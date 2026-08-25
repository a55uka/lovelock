//! PiShock's legacy share-code HTTP API.

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::{
    Command, Credentials, Error, ShockerInfo, build_http_client, expect_status, redact_api_key,
    validate_credentials, validate_duration,
};

pub(crate) const OPERATION_SUCCEEDED: &str = "Operation Succeeded.";

/// A blocking client for commands addressed through legacy share codes.
///
/// A share code identifies a command target but does not replace the username
/// and API key required by the legacy API.
pub struct LegacyClient {
    http: reqwest::blocking::Client,
    credentials: Credentials,
    sender: String,
    base_url: String,
}

impl LegacyClient {
    /// Creates a legacy share-code client after validating its credentials and sender name.
    pub fn new(credentials: Credentials, sender_name: impl Into<String>) -> Result<Self, Error> {
        Self::new_to(
            credentials,
            sender_name.into(),
            "https://do.pishock.com".to_owned(),
        )
    }

    pub(crate) fn new_to(
        credentials: Credentials,
        sender: String,
        base_url: String,
    ) -> Result<Self, Error> {
        let sender = sender.trim().to_owned();
        validate_credentials(&credentials, &sender)?;
        Ok(Self {
            http: build_http_client()?,
            credentials,
            sender,
            base_url,
        })
    }

    pub fn get_shocker_info(&self, share_code: &str) -> Result<ShockerInfo, Error> {
        let share_code = normalize_share_code(share_code)?;
        let request = ShockerInfoRequest {
            username: &self.credentials.username,
            code: &share_code,
            api_key: &self.credentials.api_key,
        };
        let response = self
            .http
            .post(format!("{}/api/GetShockerInfo", self.base_url))
            .json(&request)
            .send()
            .map_err(|_| Error::Transport)?;
        let response = expect_shocker_info_status(response)?;
        let info: ShockerInfoResponse = response.json().map_err(|_| Error::Decode {
            operation: "shocker information",
        })?;
        Ok(info.into())
    }

    /// Sends one command request to a legacy share-code target.
    pub fn send_command(&self, share_code: &str, command: Command) -> Result<(), Error> {
        let share_code = normalize_share_code(share_code)?;
        validate_command(command)?;
        let (operation, intensity, duration) = match command {
            Command::Shock {
                intensity,
                duration,
            } => (0, Some(intensity), duration),
            Command::Vibrate {
                intensity,
                duration,
            } => (1, Some(intensity), duration),
            Command::Beep { duration } => (2, None, duration),
        };
        let request = OperationRequest {
            username: &self.credentials.username,
            name: &self.sender,
            code: &share_code,
            intensity,
            duration,
            api_key: &self.credentials.api_key,
            operation,
        };
        let response = self
            .http
            .post(format!("{}/api/apioperate/", self.base_url))
            .json(&request)
            .send()
            .map_err(|_| Error::Transport)?;
        let response = expect_status(response, "command")?;
        let body = response.text().map_err(|_| Error::Transport)?;
        parse_operation_response(body.trim(), &self.credentials.api_key)
    }

    /// Shocks a share-code target at `intensity` for `duration` seconds.
    pub fn shock(&self, share_code: &str, intensity: u8, duration: u8) -> Result<(), Error> {
        self.send_command(
            share_code,
            Command::Shock {
                intensity,
                duration,
            },
        )
    }

    /// Vibrates a share-code target at `intensity` for `duration` seconds.
    pub fn vibrate(&self, share_code: &str, intensity: u8, duration: u8) -> Result<(), Error> {
        self.send_command(
            share_code,
            Command::Vibrate {
                intensity,
                duration,
            },
        )
    }

    /// Beeps a share-code target for `duration` seconds.
    pub fn beep(&self, share_code: &str, duration: u8) -> Result<(), Error> {
        self.send_command(share_code, Command::Beep { duration })
    }
}

fn normalize_share_code(share_code: &str) -> Result<String, Error> {
    let share_code = share_code.trim();
    if share_code.is_empty() {
        Err(Error::EmptyShareCode)
    } else {
        Ok(share_code.to_owned())
    }
}

fn validate_command(command: Command) -> Result<(), Error> {
    let intensity = match command {
        Command::Shock { intensity, .. } | Command::Vibrate { intensity, .. } => Some(intensity),
        Command::Beep { .. } => None,
    };
    if intensity.is_some_and(|value| !(1..=100).contains(&value)) {
        return Err(Error::InvalidIntensity);
    }
    validate_duration(match command {
        Command::Shock { duration, .. }
        | Command::Vibrate { duration, .. }
        | Command::Beep { duration } => duration,
    })
}

fn expect_shocker_info_status(
    response: reqwest::blocking::Response,
) -> Result<reqwest::blocking::Response, Error> {
    match response.status() {
        StatusCode::NOT_FOUND => Err(Error::ShareCodeNotFound),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(Error::NotAuthorized),
        _ => expect_status(response, "shocker information"),
    }
}

pub(crate) fn parse_operation_response(body: &str, api_key: &str) -> Result<(), Error> {
    match body {
        OPERATION_SUCCEEDED => Ok(()),
        "This code doesn’t exist." | "This code doesn't exist." => Err(Error::ShareCodeNotFound),
        "Not Authorized." => Err(Error::NotAuthorized),
        "Shocker is Paused, unable to send command."
        | "Shocker is Paused or does not exist. Unpause to send command." => {
            Err(Error::ShockerPaused)
        }
        "Device currently not connected." => Err(Error::DeviceOffline),
        "This share code has already been used by somebody else." | "Device in Use." => {
            Err(Error::ShareCodeInUse)
        }
        "Unknown Op, use 0 for shock, 1 for vibrate and 2 for beep." => {
            Err(Error::InvalidOperation)
        }
        "Shock not allowed." | "Vibrate not allowed." | "Beep not allowed." => {
            Err(Error::OperationNotAllowed)
        }
        message if message.starts_with("Intensity must be between 0 and ") => {
            Err(Error::IntensityRejected {
                message: redact_api_key(message, api_key),
            })
        }
        message if message.starts_with("Duration must be between 1 and ") => {
            Err(Error::DurationRejected {
                message: redact_api_key(message, api_key),
            })
        }
        message => Err(Error::OperationRejected {
            message: redact_api_key(message, api_key),
        }),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ShockerInfoRequest<'a> {
    username: &'a str,
    code: &'a str,
    #[serde(rename = "Apikey")]
    api_key: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShockerInfoResponse {
    client_id: u64,
    id: u64,
    name: String,
    paused: bool,
    max_intensity: u8,
    max_duration: u8,
    #[serde(default, alias = "isOnline")]
    online: Option<bool>,
}

impl From<ShockerInfoResponse> for ShockerInfo {
    fn from(value: ShockerInfoResponse) -> Self {
        Self {
            client_id: value.client_id,
            id: value.id,
            name: value.name,
            paused: value.paused,
            max_intensity: value.max_intensity,
            max_duration: value.max_duration,
            online: value.online,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct OperationRequest<'a> {
    username: &'a str,
    name: &'a str,
    code: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    intensity: Option<u8>,
    duration: u8,
    #[serde(rename = "Apikey")]
    api_key: &'a str,
    #[serde(rename = "Op")]
    operation: u8,
}
