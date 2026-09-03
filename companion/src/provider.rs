use std::fmt;

use crate::providers::ProviderKind;
use lovense::{Error as LovenseError, Toy};
use thiserror::Error;

/// Where to reach the local Lovense Connect/Remote HTTP server.
#[derive(Clone, PartialEq, Eq)]
pub struct LovenseSetup {
    pub domain: String,
    pub http_port: u16,
}
impl Default for LovenseSetup {
    fn default() -> Self {
        let connection = lovense::Connection::default();
        Self {
            domain: connection.domain().to_owned(),
            http_port: connection.http_port(),
        }
    }
}
impl fmt::Debug for LovenseSetup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LovenseSetup")
            .field("domain", &self.domain)
            .field("http_port", &self.http_port)
            .finish()
    }
}
impl LovenseSetup {
    pub fn normalized(&self) -> Self {
        Self {
            domain: self.domain.trim().to_owned(),
            http_port: self.http_port,
        }
    }
    pub fn present(&self) -> bool {
        !self.domain.trim().is_empty() && self.http_port != 0
    }
}

/// Configuration for the local (embedded buttplug) provider. It carries no
/// address — the engine runs in-process and scans automatically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalSetup;

/// Configuration for connecting to an externally running Intiface Central (or
/// any buttplug WebSocket server).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntifaceSetup {
    pub websocket_url: String,
}
impl Default for IntifaceSetup {
    fn default() -> Self {
        Self {
            websocket_url: "ws://127.0.0.1:12345".to_owned(),
        }
    }
}
impl IntifaceSetup {
    pub fn normalized(&self) -> Self {
        Self {
            websocket_url: self.websocket_url.trim().to_owned(),
        }
    }
    pub fn present(&self) -> bool {
        !self.websocket_url.trim().is_empty()
    }
}

/// The full provider configuration, keyed by which backend is active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderSettings {
    Lovense(LovenseSetup),
    Local(LocalSetup),
    Intiface(IntifaceSetup),
}
impl Default for ProviderSettings {
    fn default() -> Self {
        Self::Lovense(LovenseSetup::default())
    }
}
impl ProviderSettings {
    pub fn kind(&self) -> ProviderKind {
        match self {
            Self::Lovense(_) => ProviderKind::Lovense,
            Self::Local(_) => ProviderKind::Local,
            Self::Intiface(_) => ProviderKind::Intiface,
        }
    }
    pub fn present(&self) -> bool {
        match self {
            Self::Lovense(setup) => setup.present(),
            Self::Local(_) => true,
            Self::Intiface(setup) => setup.present(),
        }
    }
    pub fn normalize(&mut self) {
        match self {
            Self::Lovense(setup) => *setup = setup.normalized(),
            Self::Local(_) => {}
            Self::Intiface(setup) => *setup = setup.normalized(),
        }
    }
    pub fn lovense(&self) -> &LovenseSetup {
        match self {
            Self::Lovense(setup) => setup,
            _ => panic!("Lovense setup queried when provider is {:?}", self.kind()),
        }
    }
    pub fn lovense_mut(&mut self) -> &mut LovenseSetup {
        match self {
            Self::Lovense(setup) => setup,
            _ => panic!("Lovense setup mutated when provider is {:?}", self.kind()),
        }
    }
    pub fn intiface(&self) -> &IntifaceSetup {
        match self {
            Self::Intiface(setup) => setup,
            _ => panic!("Intiface setup queried when provider is {:?}", self.kind()),
        }
    }
    pub fn intiface_mut(&mut self) -> &mut IntifaceSetup {
        match self {
            Self::Intiface(setup) => setup,
            _ => panic!("Intiface setup mutated when provider is {:?}", self.kind()),
        }
    }
}

pub type TargetId = String;

#[derive(Clone)]
pub struct ProviderTarget {
    id: TargetId,
    name: String,
    toy: Option<Toy>,
}
impl fmt::Debug for ProviderTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderTarget")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish()
    }
}
impl PartialEq for ProviderTarget {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.name == other.name
    }
}
impl Eq for ProviderTarget {}
impl ProviderTarget {
    pub fn id(&self) -> &TargetId {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn new_any(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            toy: None,
        }
    }
    pub(crate) fn from_lovense(toy: Toy) -> Self {
        Self {
            id: toy.id.clone(),
            name: toy.name.clone(),
            toy: Some(toy),
        }
    }
    pub(crate) fn toy(&self) -> Option<&Toy> {
        self.toy.as_ref()
    }
    #[cfg(test)]
    pub(crate) fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new_any(id, name)
    }
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("Lovense: {0}")]
    Lovense(#[from] LovenseError),
    #[error("{0}")]
    Buttplug(String),
    #[error("provider setup is invalid")]
    InvalidSetup,
    #[error("provider is not connected")]
    NotConnected,
}
impl ProviderError {
    /// A plain-language explanation suitable for display in the UI. The
    /// `Display` impl above (via `#[error(...)]`) stays technical and is
    /// what gets logged.
    pub fn user_message(&self) -> String {
        match self {
            Self::Lovense(LovenseError::EmptyDomain) => {
                "Enter a connection domain first.".to_owned()
            }
            Self::Lovense(LovenseError::Transport(_)) => {
                "Can't reach the Lovense app. Is Game Mode on?".to_owned()
            }
            Self::Lovense(LovenseError::HttpStatus(_)) => {
                "Lovense app didn't respond as expected.".to_owned()
            }
            Self::Lovense(LovenseError::Decode { .. }) => {
                "Got an unexpected response from Lovense.".to_owned()
            }
            Self::Lovense(LovenseError::CommandRejected { message }) => {
                format!("Lovense rejected the request: {message}")
            }
            Self::Lovense(LovenseError::NoToysAvailable) => "No toy connected yet.".to_owned(),
            Self::Lovense(LovenseError::ToyNotFound) => {
                "That toy isn't connected anymore.".to_owned()
            }
            Self::Lovense(LovenseError::InvalidStrength)
            | Self::Lovense(LovenseError::InvalidDuration) => self.to_string(),
            Self::Buttplug(message) => message.clone(),
            Self::InvalidSetup => "Enter the required connection settings first.".to_owned(),
            Self::NotConnected => "Not connected yet.".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_present_requires_domain_and_port() {
        let mut setup = LovenseSetup::default();
        assert!(setup.present());
        setup.domain = "  ".to_owned();
        assert!(!setup.present());
        setup.domain = "127-0-0-1.lovense.club".to_owned();
        setup.http_port = 0;
        assert!(!setup.present());
    }

    #[test]
    fn setup_normalizes_whitespace() {
        let setup = LovenseSetup {
            domain: "  example.lan  ".to_owned(),
            http_port: 30010,
        };
        assert_eq!(setup.normalized().domain, "example.lan");
    }

    #[test]
    fn defaults_select_lovense_with_working_setup() {
        let settings = ProviderSettings::default();
        assert_eq!(settings.kind(), ProviderKind::Lovense);
        assert!(settings.present());
    }

    #[test]
    fn local_and_intiface_presence() {
        assert!(ProviderSettings::Local(LocalSetup).present());
        assert!(ProviderSettings::Intiface(IntifaceSetup::default()).present());
        assert!(
            !ProviderSettings::Intiface(IntifaceSetup {
                websocket_url: "  ".to_owned(),
            })
            .present()
        );
    }

    #[test]
    fn provider_kind_normalizes_its_own_fields() {
        let mut settings = ProviderSettings::Intiface(IntifaceSetup {
            websocket_url: "  ws://x  ".to_owned(),
        });
        settings.normalize();
        assert_eq!(settings.intiface().websocket_url, "ws://x");
    }
}
