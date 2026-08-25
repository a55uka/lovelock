//! Synchronous OpenShock account client.
//!
//! The client authenticates with the token endpoint, discovers owned device
//! groups over HTTP, and owns one SignalR socket for ControlV2 commands. All
//! methods block and should be called away from a GUI thread.

use std::fmt;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderValue, USER_AGENT};
use serde_json::{Value, json};
use thiserror::Error;
use tungstenite::client::IntoClientRequest;
use tungstenite::{Message, WebSocket, client_tls_with_config, stream::MaybeTlsStream};
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const MIN_DURATION_MS: u64 = 300;
const MAX_DURATION_MS: u64 = 65_535;
const RECORD_SEPARATOR: char = '\u{1e}';

/// An OpenShock API token. Debug output never includes its value.
#[derive(Clone)]
pub struct Credentials {
    token: String,
}

impl Credentials {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into().trim().to_owned(),
        }
    }
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// A shocker grouped under an owned OpenShock device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Shocker {
    pub id: String,
    pub name: String,
    pub paused: bool,
}

/// An owned OpenShock device group. A group is the selectable target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceGroup {
    pub id: String,
    pub name: String,
    pub shockers: Vec<Shocker>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TokenInfo {
    paused: bool,
    permissions: Vec<String>,
}

/// Endpoints are replaceable inside the crate for deterministic protocol tests.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Endpoints {
    http_base: String,
    websocket: String,
}

impl Endpoints {
    fn production() -> Self {
        Self {
            http_base: "https://api.openshock.app".to_owned(),
            websocket: "wss://api.openshock.app/1/hubs/user".to_owned(),
        }
    }
}

#[derive(Debug, Error, Clone, Eq, PartialEq)]
pub enum Error {
    #[error("OpenShock token must not be empty")]
    EmptyToken,
    #[error("sender name must not be empty")]
    EmptySender,
    #[error("OpenShock token was rejected")]
    AuthenticationRejected,
    #[error("OpenShock token is paused")]
    TokenPaused,
    #[error("OpenShock token lacks the shockers.use permission")]
    PermissionDenied,
    #[error("{operation} returned HTTP status {status}")]
    HttpStatus {
        operation: &'static str,
        status: u16,
    },
    #[error("OpenShock transport error")]
    Transport,
    #[error("invalid OpenShock response for {operation}")]
    Decode { operation: &'static str },
    #[error("OpenShock SignalR handshake failed")]
    Handshake,
    #[error("OpenShock command was rejected: {message}")]
    CommandRejected { message: String },
    #[error("the selected OpenShock group has no available shockers")]
    NoAvailableShockers,
    #[error("OpenShock command intensity must be between 1 and 100")]
    InvalidIntensity,
    #[error("OpenShock command duration must be between 300 and 65535 milliseconds")]
    InvalidDuration,
    #[error("OpenShock connection is closed")]
    Disconnected,
}

pub struct OpenShockClient {
    http: Client,
    credentials: Credentials,
    endpoints: Endpoints,
    worker: Arc<Mutex<Worker>>,
}

type Socket = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

struct Worker {
    socket: Option<Socket>,
    credentials: Credentials,
    sender: String,
    endpoints: Endpoints,
    next_invocation: u64,
}

impl OpenShockClient {
    pub fn connect(credentials: Credentials, sender: impl Into<String>) -> Result<Self, Error> {
        Self::connect_to(credentials, sender.into(), Endpoints::production())
    }

    fn connect_to(
        credentials: Credentials,
        sender: String,
        endpoints: Endpoints,
    ) -> Result<Self, Error> {
        let sender = sender.trim().to_owned();
        validate_credentials(&credentials, &sender)?;
        let http = build_http_client()?;
        preflight_with(&http, &credentials, &endpoints)?;
        let socket = open_socket(&credentials, &endpoints)?;
        Ok(Self {
            http,
            credentials: credentials.clone(),
            endpoints: endpoints.clone(),
            worker: Arc::new(Mutex::new(Worker {
                socket: Some(socket),
                credentials,
                sender,
                endpoints,
                next_invocation: 1,
            })),
        })
    }

    #[cfg(test)]
    fn validate_token(
        credentials: &Credentials,
        endpoints: &Endpoints,
    ) -> Result<TokenInfo, Error> {
        validate_credentials(credentials, "preflight")?;
        let http = build_http_client()?;
        preflight_with(&http, credentials, endpoints)
    }

    pub fn list_devices(&self) -> Result<Vec<DeviceGroup>, Error> {
        let response = self
            .http
            .get(format!(
                "{}/1/shockers/own",
                self.endpoints.http_base.trim_end_matches('/')
            ))
            .header(
                "Open-Shock-Token",
                HeaderValue::from_str(self.credentials.token()).map_err(|_| Error::Transport)?,
            )
            .send()
            .map_err(|_| Error::Transport)?;
        let response = expect_status(response, "device listing")?;
        let value: Value = response.json().map_err(|_| Error::Decode {
            operation: "device listing",
        })?;
        decode_groups(value)
    }

    pub fn test_sound(&self, group: &DeviceGroup, duration_ms: u64) -> Result<(), Error> {
        validate_duration(duration_ms)?;
        let shockers = available_shockers(group)?;
        let mut worker = self.worker.lock().map_err(|_| Error::Disconnected)?;
        let commands = shockers
            .iter()
            .map(|shocker| sound_command(&shocker.id, duration_ms))
            .collect();
        worker.invoke(commands)
    }

    pub fn shock(&self, group: &DeviceGroup, intensity: u8, duration_ms: u64) -> Result<(), Error> {
        validate_intensity(intensity)?;
        validate_duration(duration_ms)?;
        let shockers = available_shockers(group)?;
        let mut worker = self.worker.lock().map_err(|_| Error::Disconnected)?;
        let commands = shockers
            .iter()
            .map(|shocker| shock_command(&shocker.id, intensity, duration_ms))
            .collect();
        worker.invoke(commands)
    }

    pub fn disconnect(&self) -> Result<(), Error> {
        let mut worker = self.worker.lock().map_err(|_| Error::Disconnected)?;
        if let Some(mut socket) = worker.socket.take() {
            socket.close(None).map_err(|_| Error::Transport)?;
        }
        Ok(())
    }
}

impl Drop for OpenShockClient {
    fn drop(&mut self) {
        if let Ok(mut worker) = self.worker.lock()
            && let Some(mut socket) = worker.socket.take()
        {
            let _ = socket.close(None);
        }
    }
}

impl Worker {
    fn invoke(&mut self, commands: Vec<Value>) -> Result<(), Error> {
        let invocation = self.next_invocation.to_string();
        self.next_invocation = self.next_invocation.saturating_add(1);
        let payload = invocation_payload(&invocation, commands, &self.sender);
        if self.socket.is_none() {
            self.socket = Some(open_socket(&self.credentials, &self.endpoints)?);
        }

        let result = self.send_and_wait(&invocation, &payload);
        if matches!(
            &result,
            Err(Error::Transport | Error::Disconnected | Error::Handshake)
        ) {
            // A failed completion is ambiguous: the device may have received the
            // command before the connection dropped. Clear the socket so the
            // next command reconnects, but never replay this invocation.
            self.socket = None;
        }
        result
    }

    fn send_and_wait(&mut self, invocation: &str, payload: &Value) -> Result<(), Error> {
        let socket = self.socket.as_mut().ok_or(Error::Disconnected)?;
        let mut frame = serde_json::to_string(payload).map_err(|_| Error::Decode {
            operation: "SignalR invocation",
        })?;
        frame.push(RECORD_SEPARATOR);
        socket
            .send(Message::Text(frame.into()))
            .map_err(|_| Error::Transport)?;
        loop {
            let message = socket.read().map_err(|_| Error::Transport)?;
            let text = match message {
                Message::Text(text) => text.to_string(),
                Message::Binary(bytes) => {
                    String::from_utf8(bytes.to_vec()).map_err(|_| Error::Decode {
                        operation: "SignalR response",
                    })?
                }
                Message::Ping(payload) => {
                    socket
                        .send(Message::Pong(payload))
                        .map_err(|_| Error::Transport)?;
                    continue;
                }
                Message::Pong(_) => continue,
                Message::Close(_) => return Err(Error::Disconnected),
                _ => continue,
            };
            for record in text
                .split(RECORD_SEPARATOR)
                .filter(|record| !record.is_empty())
            {
                let value: Value = serde_json::from_str(record).map_err(|_| Error::Decode {
                    operation: "SignalR response",
                })?;
                match value.get("type").and_then(Value::as_u64) {
                    Some(3)
                        if value.get("invocationId").and_then(Value::as_str)
                            == Some(invocation) =>
                    {
                        if let Some(error) = value.get("error").and_then(Value::as_str) {
                            return Err(Error::CommandRejected {
                                message: redact(error, self.credentials.token()),
                            });
                        }
                        return Ok(());
                    }
                    Some(7) => return Err(Error::Disconnected),
                    Some(6) => continue,
                    _ => continue,
                }
            }
        }
    }
}

fn build_http_client() -> Result<Client, Error> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(user_agent())
        .build()
        .map_err(|_| Error::Transport)
}

fn user_agent() -> String {
    format!("deadlockshock-companion/{}", env!("CARGO_PKG_VERSION"))
}

fn validate_credentials(credentials: &Credentials, sender: &str) -> Result<(), Error> {
    if credentials.token.trim().is_empty() {
        return Err(Error::EmptyToken);
    }
    if sender.trim().is_empty() {
        return Err(Error::EmptySender);
    }
    Ok(())
}

fn preflight_with(
    http: &Client,
    credentials: &Credentials,
    endpoints: &Endpoints,
) -> Result<TokenInfo, Error> {
    let response = http
        .get(format!(
            "{}/2/tokens/self",
            endpoints.http_base.trim_end_matches('/')
        ))
        .header(
            "Open-Shock-Token",
            HeaderValue::from_str(credentials.token()).map_err(|_| Error::Transport)?,
        )
        .send()
        .map_err(|_| Error::Transport)?;
    let response = expect_status(response, "token authentication")?;
    let value: Value = response.json().map_err(|_| Error::Decode {
        operation: "token authentication",
    })?;
    let info = decode_token(value)?;
    if info.paused {
        return Err(Error::TokenPaused);
    }
    if !info
        .permissions
        .iter()
        .any(|permission| permission == "shockers.use")
    {
        return Err(Error::PermissionDenied);
    }
    Ok(info)
}

fn expect_status(response: Response, operation: &'static str) -> Result<Response, Error> {
    let status = response.status();
    if status.is_success() {
        Ok(response)
    } else if matches!(status.as_u16(), 401 | 403) && operation == "token authentication" {
        Err(Error::AuthenticationRejected)
    } else {
        Err(Error::HttpStatus {
            operation,
            status: status.as_u16(),
        })
    }
}

fn open_socket(credentials: &Credentials, endpoints: &Endpoints) -> Result<Socket, Error> {
    let url = Url::parse(&endpoints.websocket).map_err(|_| Error::Transport)?;
    let host = url.host_str().ok_or(Error::Transport)?;
    let port = url.port_or_known_default().ok_or(Error::Transport)?;
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|_| Error::Transport)?;
    let stream = addresses
        .into_iter()
        .find_map(|address| TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).ok())
        .ok_or(Error::Transport)?;
    stream
        .set_read_timeout(Some(COMMAND_TIMEOUT))
        .map_err(|_| Error::Transport)?;
    stream
        .set_write_timeout(Some(COMMAND_TIMEOUT))
        .map_err(|_| Error::Transport)?;
    let mut request = endpoints
        .websocket
        .clone()
        .into_client_request()
        .map_err(|_| Error::Transport)?;
    request.headers_mut().insert(
        "Open-Shock-Token",
        HeaderValue::from_str(credentials.token()).map_err(|_| Error::Transport)?,
    );
    request.headers_mut().insert(
        USER_AGENT,
        HeaderValue::from_str(&user_agent()).map_err(|_| Error::Transport)?,
    );
    let (mut socket, _) =
        client_tls_with_config(request, stream, None, None).map_err(|_| Error::Transport)?;
    let handshake = json!({"protocol":"json","version":1}).to_string() + "\u{1e}";
    socket
        .send(Message::Text(handshake.into()))
        .map_err(|_| Error::Transport)?;
    loop {
        let message = socket.read().map_err(|_| Error::Transport)?;
        let text = match message {
            Message::Text(text) => text.to_string(),
            Message::Binary(bytes) => {
                String::from_utf8(bytes.to_vec()).map_err(|_| Error::Handshake)?
            }
            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .map_err(|_| Error::Transport)?;
                continue;
            }
            Message::Close(_) => return Err(Error::Handshake),
            _ => continue,
        };
        for record in text
            .split(RECORD_SEPARATOR)
            .filter(|record| !record.is_empty())
        {
            let value: Value = serde_json::from_str(record).map_err(|_| Error::Handshake)?;
            if value.get("error").is_some() {
                return Err(Error::Handshake);
            }
            if value.as_object().is_some_and(|object| object.is_empty()) {
                return Ok(socket);
            }
        }
    }
}
fn validate_intensity(intensity: u8) -> Result<(), Error> {
    if (1..=100).contains(&intensity) {
        Ok(())
    } else {
        Err(Error::InvalidIntensity)
    }
}

fn validate_duration(duration_ms: u64) -> Result<(), Error> {
    if (MIN_DURATION_MS..=MAX_DURATION_MS).contains(&duration_ms) {
        Ok(())
    } else {
        Err(Error::InvalidDuration)
    }
}
fn available_shockers(group: &DeviceGroup) -> Result<Vec<&Shocker>, Error> {
    let shockers: Vec<&Shocker> = group
        .shockers
        .iter()
        .filter(|shocker| !shocker.paused)
        .collect();
    if shockers.is_empty() {
        Err(Error::NoAvailableShockers)
    } else {
        Ok(shockers)
    }
}

fn decode_token(value: Value) -> Result<TokenInfo, Error> {
    let object = value.get("data").unwrap_or(&value);
    let object = object.as_object().ok_or(Error::Decode {
        operation: "token authentication",
    })?;
    let control = object
        .get("shockerControl")
        .and_then(Value::as_object)
        .ok_or(Error::Decode {
            operation: "token authentication",
        })?;
    let paused = control
        .get("paused")
        .and_then(Value::as_bool)
        .ok_or(Error::Decode {
            operation: "token authentication",
        })?;
    let permissions = object
        .get("permissions")
        .map(flatten_permissions)
        .unwrap_or_default();
    Ok(TokenInfo {
        paused,
        permissions,
    })
}
fn flatten_permissions(value: &Value) -> Vec<String> {
    let mut result = Vec::new();
    match value {
        Value::String(permission) => result.push(permission.clone()),
        Value::Array(values) => {
            for value in values {
                result.extend(flatten_permissions(value));
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if value.as_bool() == Some(true) {
                    result.push(key.clone());
                }
                result.extend(flatten_permissions(value));
                if key == "use" && value.as_bool() == Some(true) {
                    result.push("shockers.use".to_owned());
                }
            }
        }
        _ => {}
    }
    result
}

fn decode_groups(value: Value) -> Result<Vec<DeviceGroup>, Error> {
    let data_value = value.get("data");
    if data_value.is_some_and(Value::is_null) {
        return Ok(Vec::new());
    }
    let data = data_value.unwrap_or(&value);
    let array = data
        .as_array()
        .or_else(|| data.get("groups").and_then(Value::as_array))
        .or_else(|| data.get("shockers").and_then(Value::as_array))
        .ok_or(Error::Decode {
            operation: "device listing",
        })?;
    let mut groups = Vec::new();
    for group in array {
        let object = group.as_object().ok_or(Error::Decode {
            operation: "device listing",
        })?;
        let id = object
            .get("id")
            .or_else(|| object.get("deviceId"))
            .and_then(string_value)
            .ok_or(Error::Decode {
                operation: "device listing",
            })?;
        let name = object
            .get("name")
            .or_else(|| object.get("deviceName"))
            .and_then(Value::as_str)
            .unwrap_or("OpenShock device")
            .to_owned();
        let shocker_values = object
            .get("shockers")
            .or_else(|| object.get("Shocker"))
            .and_then(Value::as_array)
            .ok_or(Error::Decode {
                operation: "device listing",
            })?;
        let shockers = shocker_values
            .iter()
            .map(|shocker| {
                let object = shocker.as_object().ok_or(Error::Decode {
                    operation: "device listing",
                })?;
                let id = object
                    .get("id")
                    .or_else(|| object.get("shockerId"))
                    .and_then(string_value)
                    .ok_or(Error::Decode {
                        operation: "device listing",
                    })?;
                let name = object
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("OpenShock shocker")
                    .to_owned();
                let paused = object
                    .get("paused")
                    .and_then(Value::as_bool)
                    .or_else(|| object.get("isPaused").and_then(Value::as_bool))
                    .ok_or(Error::Decode {
                        operation: "device listing",
                    })?;
                Ok(Shocker { id, name, paused })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        groups.push(DeviceGroup { id, name, shockers });
    }
    Ok(groups)
}

fn signal_command(id: &str, signal_type: u8, intensity: u8, duration_ms: u64) -> Value {
    json!({"id": id, "type": signal_type, "intensity": intensity, "duration": duration_ms, "exclusive": false})
}

fn sound_command(id: &str, duration_ms: u64) -> Value {
    signal_command(id, 3, 0, duration_ms)
}

fn shock_command(id: &str, intensity: u8, duration_ms: u64) -> Value {
    signal_command(id, 1, intensity, duration_ms)
}

fn invocation_payload(invocation: &str, commands: Vec<Value>, sender: &str) -> Value {
    json!({"type": 1, "invocationId": invocation, "target": "ControlV2", "arguments": [commands, sender]})
}

fn string_value(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|number| number.to_string()))
}
fn redact(message: &str, token: &str) -> String {
    message.replace(token, "[REDACTED]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    use mockito::{Matcher, Server};
    use tungstenite::handshake::server::Request;

    const TOKEN: &str = "open-shock-secret";
    const SENDER: &str = "deadlockshock-companion";

    fn endpoints(http_base: String, websocket: impl Into<String>) -> Endpoints {
        Endpoints {
            http_base,
            websocket: websocket.into(),
        }
    }

    fn text(message: Message) -> String {
        match message {
            Message::Text(value) => value.to_string(),
            other => panic!("expected text message, got {other:?}"),
        }
    }

    #[test]
    fn credentials_debug_redacts_token() {
        let credentials = Credentials::new("secret-token");
        let debug = format!("{credentials:?}");
        assert!(!debug.contains("secret-token"));
        assert!(debug.contains("REDACTED"));
    }
    #[test]
    fn command_error_redacts_token() {
        assert_eq!(
            redact(&format!("token {TOKEN} rejected"), TOKEN),
            "token [REDACTED] rejected"
        );
    }

    #[test]
    fn duration_boundaries_are_validated() {
        assert!(validate_duration(300).is_ok());
        assert!(validate_duration(65_535).is_ok());
        assert_eq!(validate_duration(299), Err(Error::InvalidDuration));
        assert_eq!(validate_duration(65_536), Err(Error::InvalidDuration));
    }
    #[test]
    fn intensity_boundaries_are_validated() {
        assert!(validate_intensity(1).is_ok());
        assert!(validate_intensity(100).is_ok());
        assert_eq!(validate_intensity(0), Err(Error::InvalidIntensity));
    }

    #[test]
    fn shock_command_has_exact_control_v2_shape() {
        assert_eq!(
            shock_command("shocker", 42, 1_234),
            json!({
                "id": "shocker",
                "type": 1,
                "intensity": 42,
                "duration": 1_234,
                "exclusive": false
            })
        );
    }

    #[test]
    fn available_shockers_filter_paused_targets_and_reject_empty_groups() {
        let group = DeviceGroup {
            id: "group".to_owned(),
            name: "Group".to_owned(),
            shockers: vec![
                Shocker {
                    id: "active".to_owned(),
                    name: "Active".to_owned(),
                    paused: false,
                },
                Shocker {
                    id: "paused".to_owned(),
                    name: "Paused".to_owned(),
                    paused: true,
                },
            ],
        };
        let available = available_shockers(&group).expect("active shocker");
        assert_eq!(
            available
                .iter()
                .map(|shocker| shocker.id.as_str())
                .collect::<Vec<_>>(),
            vec!["active"]
        );
        let empty = DeviceGroup {
            id: "empty".to_owned(),
            name: "Empty".to_owned(),
            shockers: vec![Shocker {
                id: "paused".to_owned(),
                name: "Paused".to_owned(),
                paused: true,
            }],
        };
        assert_eq!(available_shockers(&empty), Err(Error::NoAvailableShockers));
    }

    #[test]
    fn shock_validates_before_socket_invocation() {
        let client = OpenShockClient {
            http: build_http_client().expect("http client"),
            credentials: Credentials::new(TOKEN),
            endpoints: Endpoints::production(),
            worker: Arc::new(Mutex::new(Worker {
                socket: None,
                credentials: Credentials::new(TOKEN),
                sender: SENDER.to_owned(),
                endpoints: Endpoints::production(),
                next_invocation: 1,
            })),
        };
        let active = DeviceGroup {
            id: "group".to_owned(),
            name: "Group".to_owned(),
            shockers: vec![Shocker {
                id: "active".to_owned(),
                name: "Active".to_owned(),
                paused: false,
            }],
        };
        assert_eq!(
            client.shock(&active, 0, 1_000),
            Err(Error::InvalidIntensity)
        );
        assert_eq!(client.shock(&active, 50, 299), Err(Error::InvalidDuration));
        let paused = DeviceGroup {
            id: "paused".to_owned(),
            name: "Paused".to_owned(),
            shockers: vec![Shocker {
                id: "paused".to_owned(),
                name: "Paused".to_owned(),
                paused: true,
            }],
        };
        assert_eq!(
            client.shock(&paused, 50, 1_000),
            Err(Error::NoAvailableShockers)
        );
        assert!(client.worker.lock().expect("worker lock").socket.is_none());
    }

    #[test]
    fn legacy_envelope_decodes_groups() {
        let value = json!({"success":true,"data":[{"id":"g","name":"Group","shockers":[{"id":"s","name":"S","paused":true}]}]});
        let groups = decode_groups(value).expect("groups");
        assert_eq!(groups[0].shockers[0].id, "s");
        assert!(groups[0].shockers[0].paused);
    }
    #[test]
    fn token_decoder_requires_boolean_pause_state() {
        for value in [
            json!({"data":{"permissions":[]}}),
            json!({"data":{"shockerControl":{}}}),
            json!({"data":{"shockerControl":{"paused":null}}}),
            json!({"data":{"shockerControl":{"paused":"false"}}}),
        ] {
            assert_eq!(
                decode_token(value),
                Err(Error::Decode {
                    operation: "token authentication"
                })
            );
        }
    }

    #[test]
    fn group_decoder_accepts_both_pause_spellings_and_rejects_invalid_state() {
        for (key, pause) in [("paused", true), ("isPaused", false)] {
            let mut value = json!({
                "data": [{
                    "id": "g",
                    "name": "Group",
                    "shockers": [{"id": "s", "name": "S"}]
                }]
            });
            value["data"][0]["shockers"][0][key] = json!(pause);
            let groups = decode_groups(value).expect("groups");
            assert_eq!(groups[0].shockers[0].paused, pause);
        }
        for pause in [
            json!({}),
            json!({"paused": null}),
            json!({"isPaused": "false"}),
        ] {
            let mut payload = json!({
                "data": [{
                    "id": "g",
                    "name": "Group",
                    "shockers": [{"id": "s", "name": "S"}]
                }]
            });
            if let Some(object) = pause.as_object() {
                for (key, pause_value) in object {
                    payload["data"][0]["shockers"][0][key] = pause_value.clone();
                }
            }
            assert_eq!(
                decode_groups(payload),
                Err(Error::Decode {
                    operation: "device listing"
                })
            );
        }
    }
    #[test]
    fn token_decoder_checks_nested_pause_state() {
        let value = json!({"data":{"id":"t","permissions":["shockers.use"],"shockerControl":{"paused":true}}});
        assert!(decode_token(value).expect("token").paused);
    }

    #[test]
    fn null_discovery_data_is_empty() {
        assert!(
            decode_groups(json!({"data":null}))
                .expect("groups")
                .is_empty()
        );
    }

    #[test]
    fn control_v2_invocation_has_complete_shape() {
        let command = sound_command("shocker", 300);
        let frame = invocation_payload("7", vec![command], "sender");
        assert_eq!(frame["type"], 1);
        assert_eq!(frame["invocationId"], "7");
        assert_eq!(frame["target"], "ControlV2");
        assert_eq!(frame["arguments"][0][0]["type"], 3);
        assert_eq!(frame["arguments"][0][0]["intensity"], 0);
        assert_eq!(frame["arguments"][0][0]["duration"], 300);
        assert_eq!(frame["arguments"][0][0]["exclusive"], false);
        assert!(frame["arguments"][0][0].get("customName").is_none());
        assert_eq!(frame["arguments"][1], "sender");
    }

    #[test]
    fn preflight_rejects_paused_and_permissionless_tokens() {
        let cases = [
            (
                r#"{"id":"token","permissions":["shockers.use"],"shockerControl":{"paused":true}}"#,
                Error::TokenPaused,
            ),
            (
                r#"{"id":"token","permissions":[],"shockerControl":{"paused":false}}"#,
                Error::PermissionDenied,
            ),
        ];

        for (body, expected) in cases {
            let mut server = Server::new();
            let token = server
                .mock("GET", "/2/tokens/self")
                .match_header("open-shock-token", TOKEN)
                .match_header("user-agent", Matcher::Exact(user_agent()))
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(body)
                .expect(1)
                .create();
            let result = OpenShockClient::validate_token(
                &Credentials::new(TOKEN),
                &endpoints(server.url(), "ws://127.0.0.1:1/unused"),
            );

            assert_eq!(result, Err(expected));
            token.assert();
        }
    }

    #[test]
    #[allow(clippy::result_large_err)]
    fn client_authenticates_discovers_and_sends_exact_signalr_invocation() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind websocket server");
        let address = listener.local_addr().expect("websocket address");
        let websocket = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept websocket client");
            let mut socket = tungstenite::accept_hdr(stream, |request: &Request, response| {
                assert_eq!(
                    request
                        .headers()
                        .get("open-shock-token")
                        .and_then(|value| value.to_str().ok()),
                    Some(TOKEN)
                );
                assert_eq!(
                    request
                        .headers()
                        .get("user-agent")
                        .and_then(|value| value.to_str().ok()),
                    Some(user_agent().as_str())
                );
                Ok(response)
            })
            .expect("upgrade websocket");

            let handshake = text(socket.read().expect("read SignalR handshake"));
            assert!(handshake.ends_with(RECORD_SEPARATOR));
            assert_eq!(
                serde_json::from_str::<Value>(handshake.trim_end_matches(RECORD_SEPARATOR))
                    .expect("decode SignalR handshake"),
                json!({"protocol": "json", "version": 1})
            );
            socket
                .send(Message::Text("{}\u{1e}".into()))
                .expect("complete SignalR handshake");

            let frame = text(socket.read().expect("read ControlV2 invocation"));
            assert!(frame.ends_with(RECORD_SEPARATOR));
            let invocation: Value = serde_json::from_str(frame.trim_end_matches(RECORD_SEPARATOR))
                .expect("decode ControlV2 invocation");
            let invocation_id = invocation["invocationId"].as_str().expect("invocation id");
            let completion = json!({"type": 3, "invocationId": invocation_id}).to_string()
                + &RECORD_SEPARATOR.to_string();
            socket
                .send(Message::Text(completion.into()))
                .expect("complete ControlV2 invocation");
            assert!(matches!(
                socket.read().expect("read graceful close"),
                Message::Close(_)
            ));
            invocation
        });

        let mut server = Server::new();
        let token = server
            .mock("GET", "/2/tokens/self")
            .match_header("open-shock-token", TOKEN)
            .match_header("user-agent", Matcher::Exact(user_agent()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"id":"token","permissions":["shockers.use"],"shockerControl":{"paused":false}}"#,
            )
            .expect(1)
            .create();
        let discovery = server
            .mock("GET", "/1/shockers/own")
            .match_header("open-shock-token", TOKEN)
            .match_header("user-agent", Matcher::Exact(user_agent()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"message":"","data":[{"id":"device","name":"Device","shockers":[{"id":"active","name":"Active","isPaused":false},{"id":"paused","name":"Paused","isPaused":true}]}]}"#,
            )
            .expect(1)
            .create();
        let client = OpenShockClient::connect_to(
            Credentials::new(" \topen-shock-secret \n"),
            " \tdeadlockshock-companion \n".to_owned(),
            endpoints(server.url(), format!("ws://{address}/1/hubs/user")),
        )
        .expect("connect OpenShock client");
        let groups = client.list_devices().expect("discover devices");
        client.shock(&groups[0], 42, 1_000).expect("send shock");
        client.disconnect().expect("disconnect OpenShock client");

        let invocation = websocket.join().expect("join websocket server");
        assert_eq!(invocation["type"], 1);
        assert_eq!(invocation["target"], "ControlV2");
        assert_eq!(invocation["arguments"][1], SENDER);
        assert_eq!(
            invocation["arguments"][0],
            json!([{
                "id": "active",
                "type": 1,
                "intensity": 42,
                "duration": 1_000,
                "exclusive": false
            }])
        );
        token.assert();
        discovery.assert();
    }

    #[test]
    fn ambiguous_transport_failure_clears_socket_without_replaying_command() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind websocket server");
        let address = listener.local_addr().expect("local address");
        let websocket = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept websocket client");
            let mut socket = tungstenite::accept(stream).expect("upgrade websocket");
            let handshake = text(socket.read().expect("read SignalR handshake"));
            assert!(handshake.ends_with(RECORD_SEPARATOR));
            socket
                .send(Message::Text("{}\u{1e}".into()))
                .expect("complete SignalR handshake");
            let frame = text(socket.read().expect("read ControlV2 invocation"));
            let invocation =
                serde_json::from_str::<Value>(frame.trim_end_matches(RECORD_SEPARATOR))
                    .expect("decode ControlV2 invocation");
            drop(socket);
            invocation
        });

        let mut server = Server::new();
        let token = server
        .mock("GET", "/2/tokens/self")
        .match_header("open-shock-token", TOKEN)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"id":"token","permissions":["shockers.use"],"shockerControl":{"paused":false}}"#,
        )
        .expect(1)
        .create();
        let client = OpenShockClient::connect_to(
            Credentials::new(TOKEN),
            SENDER.to_owned(),
            endpoints(server.url(), format!("ws://{address}/1/hubs/user")),
        )
        .expect("connect OpenShock client");
        let group = DeviceGroup {
            id: "device".to_owned(),
            name: "Device".to_owned(),
            shockers: vec![Shocker {
                id: "active".to_owned(),
                name: "Active".to_owned(),
                paused: false,
            }],
        };

        assert_eq!(client.shock(&group, 42, 1_000), Err(Error::Transport));
        assert!(client.worker.lock().expect("worker lock").socket.is_none());
        client.disconnect().expect("disconnect without reconnect");
        let invocation = websocket.join().expect("join websocket server");
        assert_eq!(invocation["arguments"][0].as_array().map(Vec::len), Some(1));
        token.assert();
    }
}
