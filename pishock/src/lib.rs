//! Synchronous clients for PiShock's WebSocket and legacy API surfaces.
//!
//! [`WebSocketClient`] authenticates an account, discovers its owned [`Device`]s,
//! and commands their paired [`Shocker`]s through the WebSocket API.
//! [`LegacyClient`] addresses one shocker through a share code. A share code is a
//! command target, not a replacement for account credentials. Every client
//! method blocks the calling thread, so GUI applications should call it away
//! from the UI thread.
//!
//! # Example
//!
//! ```no_run
//! use pishock::{Credentials, LegacyClient, WebSocketClient};
//!
//! let credentials = Credentials::new("username", "api-key");
//! let account = WebSocketClient::connect(
//!     credentials.clone(),
//!     "deadlockshock-companion",
//! )?;
//! let devices = account.list_devices()?;
//! if let Some(device) = devices.first() {
//!     account.beep_device(device, 1)?;
//! }
//!
//! let legacy = LegacyClient::new(credentials, "deadlockshock-companion")?;
//! legacy.beep("share-code", 1)?;
//! # Ok::<(), pishock::Error>(())
//! ```

use std::fmt;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use thiserror::Error;

pub mod legacy;
pub mod websocket;

pub use legacy::LegacyClient;
pub use websocket::WebSocketClient;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_DURATION: u8 = 15;

/// A PiShock username and API key.
///
/// Debug formatting always redacts the API key.
#[derive(Clone)]
pub struct Credentials {
    pub(crate) username: String,
    pub(crate) api_key: String,
}

impl Credentials {
    /// Creates credentials. Values are validated when constructing either client.
    pub fn new(username: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            username: username.into().trim().to_owned(),
            api_key: api_key.into().trim().to_owned(),
        }
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("username", &self.username)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

/// An owned PiShock hub and the shockers paired to it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Device {
    /// The hub's client ID.
    pub client_id: u64,
    /// The hub's display name.
    pub name: String,
    /// The owning user's ID.
    pub user_id: u64,
    /// The owning user's username.
    pub username: String,
    /// Shockers paired to this hub.
    pub shockers: Vec<Shocker>,
}

/// A shocker paired to an owned hub.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Shocker {
    /// The shocker's display name.
    pub name: String,
    /// The shocker's ID within PiShock.
    pub shocker_id: u64,
    /// Whether commands to the shocker are paused.
    pub is_paused: bool,
}

/// Information and share limits for a share-code command target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShockerInfo {
    /// The client ID of the hub hosting the shocker.
    pub client_id: u64,
    /// The shocker's ID within PiShock.
    pub id: u64,
    /// The shocker's display name.
    pub name: String,
    /// Whether commands are paused.
    pub paused: bool,
    /// The maximum intensity allowed by the share code.
    pub max_intensity: u8,
    /// The maximum duration allowed by the share code.
    pub max_duration: u8,
    /// Whether the hosting hub is connected, when reported by PiShock.
    pub online: Option<bool>,
}

/// An operation sent to a shocker through a legacy share code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    /// Shock at `intensity` for `duration` seconds.
    Shock { intensity: u8, duration: u8 },
    /// Vibrate at `intensity` for `duration` seconds.
    Vibrate { intensity: u8, duration: u8 },
    /// Beep for `duration` seconds.
    Beep { duration: u8 },
}

/// An error produced by validation or a PiShock API request.
///
/// Transport and decoding errors intentionally omit request URLs so API keys
/// embedded in query strings cannot be exposed through formatting.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum Error {
    /// The username was empty.
    #[error("username must not be empty")]
    EmptyUsername,
    /// The API key was empty.
    #[error("API key must not be empty")]
    EmptyApiKey,
    /// The operation-log sender name was empty.
    #[error("sender name must not be empty")]
    EmptySender,
    /// A share code was empty.
    #[error("share code must not be empty")]
    EmptyShareCode,
    /// An intensity was outside 1 through 100.
    #[error("intensity must be between 1 and 100")]
    InvalidIntensity,
    /// A legacy command duration was outside 1 through 15 seconds.
    #[error("duration must be between 1 and 15 seconds")]
    InvalidDuration,
    /// A WebSocket command duration was outside 300 through 65535 milliseconds.
    #[error("WebSocket duration must be between 300 and 65535 milliseconds")]
    InvalidWebSocketDuration,
    /// The API key authentication request was rejected.
    #[error("PiShock authentication was rejected")]
    AuthenticationRejected,
    /// The share code does not exist.
    #[error("the share code does not exist")]
    ShareCodeNotFound,
    /// The username or API key was not authorized.
    #[error("PiShock did not authorize the request")]
    NotAuthorized,
    /// The target shocker is paused or unavailable.
    #[error("the target shocker is paused or unavailable")]
    ShockerPaused,
    /// The target hub is offline.
    #[error("the target PiShock hub is not connected")]
    DeviceOffline,
    /// The target device or share code is already in use elsewhere.
    #[error("the target PiShock device is already in use")]
    ShareCodeInUse,
    /// PiShock rejected the command operation code.
    #[error("PiShock rejected the operation code")]
    InvalidOperation,
    /// The share code does not allow the selected operation.
    #[error("the share code does not allow this operation")]
    OperationNotAllowed,
    /// The selected hub has no unpaused shockers to command.
    #[error("the selected PiShock hub has no available shockers")]
    NoAvailableShockers,
    /// PiShock rejected the requested intensity against share limits.
    #[error("PiShock rejected the intensity: {message}")]
    IntensityRejected {
        /// The rejection returned by PiShock.
        message: String,
    },
    /// PiShock rejected the requested duration against share limits.
    #[error("PiShock rejected the duration: {message}")]
    DurationRejected {
        /// The rejection returned by PiShock.
        message: String,
    },
    /// PiShock returned an otherwise unknown legacy operation rejection.
    #[error("PiShock rejected the operation: {message}")]
    OperationRejected {
        /// The trimmed rejection returned by PiShock.
        message: String,
    },
    /// The WebSocket API rejected an owned-device command.
    #[error("PiShock rejected the device command: {message}")]
    WebSocketRejected {
        /// The rejection returned by PiShock, with the API key redacted.
        message: String,
    },
    /// A server returned a non-success HTTP status.
    #[error("{operation} returned HTTP status {status}")]
    HttpStatus {
        /// The operation that failed, without its URL.
        operation: &'static str,
        /// The numeric HTTP status.
        status: u16,
    },
    /// A request could not be sent or its response could not be read.
    #[error("PiShock transport error")]
    Transport,
    /// A successful response did not match PiShock's documented schema.
    #[error("invalid PiShock response for {operation}")]
    Decode {
        /// The operation whose response could not be decoded.
        operation: &'static str,
    },
}

pub(crate) fn build_http_client() -> Result<Client, Error> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| Error::Transport)
}

pub(crate) fn validate_credentials(credentials: &Credentials, sender: &str) -> Result<(), Error> {
    if credentials.username.trim().is_empty() {
        return Err(Error::EmptyUsername);
    }
    if credentials.api_key.trim().is_empty() {
        return Err(Error::EmptyApiKey);
    }
    if sender.trim().is_empty() {
        return Err(Error::EmptySender);
    }
    Ok(())
}

pub(crate) fn validate_duration(duration: u8) -> Result<(), Error> {
    if (1..=MAX_DURATION).contains(&duration) {
        Ok(())
    } else {
        Err(Error::InvalidDuration)
    }
}

pub(crate) fn expect_status(
    response: Response,
    operation: &'static str,
) -> Result<Response, Error> {
    let status = response.status();
    if status.is_success() {
        Ok(response)
    } else if operation == "authentication"
        && matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
    {
        Err(Error::AuthenticationRejected)
    } else {
        Err(Error::HttpStatus {
            operation,
            status: status.as_u16(),
        })
    }
}

pub(crate) fn redact_api_key(message: &str, api_key: &str) -> String {
    message.replace(api_key, "[REDACTED]")
}

#[cfg(test)]
mod tests;
