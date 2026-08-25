//! PiShock's account-authenticated device discovery and WebSocket API.

use std::net::{TcpStream, ToSocketAddrs};

use serde::{Deserialize, Serialize};
use tungstenite::client::IntoClientRequest;
use tungstenite::{Message, client_tls_with_config};
use url::Url;

use crate::{
    COMMAND_TIMEOUT, CONNECT_TIMEOUT, Credentials, Device, Error, Shocker, build_http_client,
    expect_status, redact_api_key, validate_credentials, validate_duration,
};

const MIN_WEBSOCKET_DURATION_MS: u64 = 300;
const MAX_WEBSOCKET_DURATION_MS: u64 = u16::MAX as u64;

/// A connected client for account-owned devices and WebSocket commands.
///
/// Connecting authenticates the API key once and stores the resulting user ID.
/// All methods block the calling thread.
pub struct WebSocketClient {
    http: reqwest::blocking::Client,
    credentials: Credentials,
    sender: String,
    user_id: u64,
    pub(crate) urls: WebSocketUrls,
}

impl WebSocketClient {
    /// Validates credentials and sender name, authenticates, and extracts the user ID.
    pub fn connect(
        credentials: Credentials,
        sender_name: impl Into<String>,
    ) -> Result<Self, Error> {
        Self::connect_to(credentials, sender_name.into(), WebSocketUrls::production())
    }

    pub(crate) fn connect_to(
        credentials: Credentials,
        sender: String,
        urls: WebSocketUrls,
    ) -> Result<Self, Error> {
        let sender = sender.trim().to_owned();
        validate_credentials(&credentials, &sender)?;
        let http = build_http_client()?;
        let response = http
            .get(format!("{}/Auth/GetUserIfAPIKeyValid", urls.auth))
            .query(&[
                ("apikey", credentials.api_key.as_str()),
                ("username", credentials.username.as_str()),
            ])
            .send()
            .map_err(|_| Error::Transport)?;
        let response = expect_status(response, "authentication")?;
        let auth: AuthResponse = response.json().map_err(|_| Error::Decode {
            operation: "authentication",
        })?;

        Ok(Self {
            http,
            credentials,
            sender,
            user_id: auth.user_id,
            urls,
        })
    }

    /// Lists the authenticated user's owned hubs and their paired shockers.
    pub fn list_devices(&self) -> Result<Vec<Device>, Error> {
        let response = self
            .http
            .get(format!("{}/PiShock/GetUserDevices", self.urls.platform))
            .query(&[
                ("UserId", self.user_id.to_string()),
                ("Token", self.credentials.api_key.clone()),
                ("api", "true".to_owned()),
            ])
            .send()
            .map_err(|_| Error::Transport)?;
        let response = expect_status(response, "device listing")?;
        let devices: Vec<DeviceResponse> = response.json().map_err(|_| Error::Decode {
            operation: "device listing",
        })?;
        Ok(devices.into_iter().map(Device::from).collect())
    }

    /// Lists owned hubs and returns the one with `client_id`, if present.
    pub fn get_device(&self, client_id: u64) -> Result<Option<Device>, Error> {
        Ok(self
            .list_devices()?
            .into_iter()
            .find(|device| device.client_id == client_id))
    }

    /// Beeps every unpaused shocker paired to an owned hub.
    pub fn beep_device(&self, device: &Device, duration: u8) -> Result<(), Error> {
        validate_duration(duration)?;
        let payload = self.owned_device_payload(device, "b", 0, u64::from(duration) * 1_000)?;
        self.publish(&payload)
    }

    /// Shocks every unpaused shocker paired to an owned hub.
    ///
    /// `duration_ms` is expressed in milliseconds and must be between
    /// `MIN_WEBSOCKET_DURATION_MS` and `MAX_WEBSOCKET_DURATION_MS` inclusive,
    /// matching the WebSocket command's unsigned 16-bit duration field.
    pub fn shock_device(
        &self,
        device: &Device,
        intensity: u8,
        duration_ms: u64,
    ) -> Result<(), Error> {
        if !(1..=100).contains(&intensity) {
            return Err(Error::InvalidIntensity);
        }
        if !(MIN_WEBSOCKET_DURATION_MS..=MAX_WEBSOCKET_DURATION_MS).contains(&duration_ms) {
            return Err(Error::InvalidWebSocketDuration);
        }
        let payload = self.owned_device_payload(device, "s", intensity, duration_ms)?;
        self.publish(&payload)
    }

    fn owned_device_payload(
        &self,
        device: &Device,
        mode: &'static str,
        intensity: u8,
        duration: u64,
    ) -> Result<String, Error> {
        let shockers: Vec<&Shocker> = device
            .shockers
            .iter()
            .filter(|shocker| !shocker.is_paused)
            .collect();
        if shockers.is_empty() {
            return Err(Error::NoAvailableShockers);
        }

        serde_json::to_string(&WebSocketRequest {
            operation: "PUBLISH",
            publish_commands: shockers
                .into_iter()
                .map(|shocker| PublishCommand {
                    target: format!("c{}-ops", device.client_id),
                    body: OwnedCommandBody {
                        id: shocker.shocker_id,
                        mode,
                        intensity,
                        duration,
                        repeating: true,
                        log: CommandLog {
                            user_id: self.user_id,
                            command_type: "api",
                            warning: false,
                            hold: false,
                            origin: &self.sender,
                        },
                    },
                })
                .collect(),
        })
        .map_err(|_| Error::Decode {
            operation: "owned device command",
        })
    }

    fn publish(&self, payload: &str) -> Result<(), Error> {
        let mut url = Url::parse(&self.urls.websocket).map_err(|_| Error::Decode {
            operation: "WebSocket URL",
        })?;
        url.query_pairs_mut()
            .append_pair("Username", &self.credentials.username)
            .append_pair("ApiKey", &self.credentials.api_key);

        let host = url.host_str().ok_or(Error::Decode {
            operation: "WebSocket URL",
        })?;
        let port = url.port_or_known_default().ok_or(Error::Decode {
            operation: "WebSocket URL",
        })?;
        let addresses = (host, port)
            .to_socket_addrs()
            .map_err(|_| Error::Transport)?;
        let stream = connect_with_timeout(addresses)?;
        stream
            .set_read_timeout(Some(COMMAND_TIMEOUT))
            .map_err(|_| Error::Transport)?;
        stream
            .set_write_timeout(Some(COMMAND_TIMEOUT))
            .map_err(|_| Error::Transport)?;

        let request = url
            .as_str()
            .into_client_request()
            .map_err(|_| Error::Transport)?;
        let (mut socket, _) =
            client_tls_with_config(request, stream, None, None).map_err(|_| Error::Transport)?;
        socket
            .send(Message::Text(payload.to_owned().into()))
            .map_err(|_| Error::Transport)?;

        let response = socket.read().map_err(|_| Error::Transport)?;
        let response = match response {
            Message::Text(text) => serde_json::from_str::<WebSocketResponse>(&text),
            Message::Binary(bytes) => serde_json::from_slice::<WebSocketResponse>(&bytes),
            _ => {
                return Err(Error::Decode {
                    operation: "owned device command",
                });
            }
        }
        .map_err(|_| Error::Decode {
            operation: "owned device command",
        })?;
        let _ = socket.close(None);

        if response.is_error {
            Err(Error::WebSocketRejected {
                message: redact_api_key(&response.message, &self.credentials.api_key),
            })
        } else {
            Ok(())
        }
    }
}

fn connect_with_timeout(
    addresses: impl Iterator<Item = std::net::SocketAddr>,
) -> Result<TcpStream, Error> {
    for address in addresses {
        if let Ok(stream) = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
            return Ok(stream);
        }
    }
    Err(Error::Transport)
}

#[derive(Clone)]
pub(crate) struct WebSocketUrls {
    pub(crate) auth: String,
    pub(crate) platform: String,
    pub(crate) websocket: String,
}

impl WebSocketUrls {
    fn production() -> Self {
        Self {
            auth: "https://auth.pishock.com".to_owned(),
            platform: "https://ps.pishock.com".to_owned(),
            websocket: "wss://broker.pishock.com/v2".to_owned(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AuthResponse {
    #[serde(rename = "UserID", alias = "UserId", alias = "userId")]
    user_id: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceResponse {
    client_id: u64,
    name: String,
    user_id: u64,
    username: String,
    shockers: Vec<ShockerResponse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShockerResponse {
    name: String,
    shocker_id: u64,
    is_paused: bool,
}

impl From<DeviceResponse> for Device {
    fn from(value: DeviceResponse) -> Self {
        Self {
            client_id: value.client_id,
            name: value.name,
            user_id: value.user_id,
            username: value.username,
            shockers: value.shockers.into_iter().map(Shocker::from).collect(),
        }
    }
}

impl From<ShockerResponse> for Shocker {
    fn from(value: ShockerResponse) -> Self {
        Self {
            name: value.name,
            shocker_id: value.shocker_id,
            is_paused: value.is_paused,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct WebSocketRequest<'a> {
    operation: &'static str,
    publish_commands: Vec<PublishCommand<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PublishCommand<'a> {
    target: String,
    body: OwnedCommandBody<'a>,
}

#[derive(Serialize)]
struct OwnedCommandBody<'a> {
    id: u64,
    #[serde(rename = "m")]
    mode: &'static str,
    #[serde(rename = "i")]
    intensity: u8,
    #[serde(rename = "d")]
    duration: u64,
    #[serde(rename = "r")]
    repeating: bool,
    #[serde(rename = "l")]
    log: CommandLog<'a>,
}

#[derive(Serialize)]
struct CommandLog<'a> {
    #[serde(rename = "u")]
    user_id: u64,
    #[serde(rename = "ty")]
    command_type: &'static str,
    #[serde(rename = "w")]
    warning: bool,
    #[serde(rename = "h")]
    hold: bool,
    #[serde(rename = "o")]
    origin: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WebSocketResponse {
    is_error: bool,
    message: String,
}
