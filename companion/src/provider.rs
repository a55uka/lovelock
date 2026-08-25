use std::fmt;
use std::time::Duration;

use crate::action::{ActionKind, ResolvedAction};
use lovense::{Connection as LovenseConnection, Error as LovenseError, LovenseClient, Toy};
use openshock::{
    Credentials as OpenShockCredentials, DeviceGroup, Error as OpenShockError, OpenShockClient,
};
use pishock::{Credentials as PiShockCredentials, Device, Error as PiShockError, WebSocketClient};
use thiserror::Error;

pub const SENDER_NAME: &str = "deadlockshock-companion";
pub const TEST_SOUND_DURATION: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProviderKind {
    #[default]
    PiShock,
    OpenShock,
    Lovense,
}
impl ProviderKind {
    pub fn descriptor(self) -> ProviderDescriptor {
        match self {
            Self::PiShock => ProviderDescriptor {
                kind: self,
                label: "PiShock",
                setup_label: "PiShock setup",
                setup_help: "PiShock account username and API key.",
                setup_kind: SetupKind::UserAndSecret,
                target_noun: "Device group",
                target_policy: TargetPolicy::Required,
                test_action: Some(TestActionKind::Sound),
                action_kind: ActionKind::Shock,
            },
            Self::OpenShock => ProviderDescriptor {
                kind: self,
                label: "OpenShock",
                setup_label: "OpenShock setup",
                setup_help: "OpenShock API token with shockers.use permission.",
                setup_kind: SetupKind::Secret,
                target_noun: "Device group",
                target_policy: TargetPolicy::Required,
                test_action: Some(TestActionKind::Sound),
                action_kind: ActionKind::Shock,
            },
            Self::Lovense => ProviderDescriptor {
                kind: self,
                label: "Lovense",
                setup_label: "Lovense setup",
                setup_help: "Enable Game Mode in the Lovense Remote app on the same PC, then Test connection. Use a custom domain/port if your toy is paired to a phone instead.",
                setup_kind: SetupKind::ProviderSpecific,
                target_noun: "Toy",
                target_policy: TargetPolicy::Optional,
                test_action: Some(TestActionKind::Pulse),
                action_kind: ActionKind::Vibrate,
            },
        }
    }
    pub fn label(self) -> &'static str {
        self.descriptor().label
    }
    pub fn action_kind(self) -> ActionKind {
        self.descriptor().action_kind
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TestActionKind {
    Sound,
    Pulse,
}
impl TestActionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sound => "Send test sound",
            Self::Pulse => "Send test vibration",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupKind {
    None,
    Secret,
    UserAndSecret,
    ProviderSpecific,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetPolicy {
    Required,
    Optional,
    None,
}
impl TargetPolicy {
    pub fn accepts(self, target_present: bool) -> bool {
        match self {
            Self::Required => target_present,
            Self::Optional => true,
            Self::None => !target_present,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderDescriptor {
    pub kind: ProviderKind,
    pub label: &'static str,
    pub setup_label: &'static str,
    pub setup_help: &'static str,
    pub setup_kind: SetupKind,
    pub target_noun: &'static str,
    pub target_policy: TargetPolicy,
    pub test_action: Option<TestActionKind>,
    pub action_kind: ActionKind,
}
pub const SUPPORTED_PROVIDERS: [ProviderKind; 3] = [
    ProviderKind::PiShock,
    ProviderKind::OpenShock,
    ProviderKind::Lovense,
];

#[derive(Clone, Default, PartialEq, Eq)]
pub struct PiShockSetup {
    pub username: String,
    pub api_key: String,
}
impl fmt::Debug for PiShockSetup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PiShockSetup")
            .field("username", &self.username)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}
impl PiShockSetup {
    pub fn normalized(&self) -> Self {
        Self {
            username: self.username.trim().to_owned(),
            api_key: self.api_key.trim().to_owned(),
        }
    }
    pub fn present(&self) -> bool {
        !self.username.trim().is_empty() && !self.api_key.trim().is_empty()
    }
}
#[derive(Clone, Default, PartialEq, Eq)]
pub struct OpenShockSetup {
    pub token: String,
}
impl fmt::Debug for OpenShockSetup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenShockSetup")
            .field("token", &"[REDACTED]")
            .finish()
    }
}
impl OpenShockSetup {
    pub fn normalized(&self) -> Self {
        Self {
            token: self.token.trim().to_owned(),
        }
    }
    pub fn present(&self) -> bool {
        !self.token.trim().is_empty()
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct LovenseSetup {
    pub domain: String,
    pub http_port: u16,
}
impl Default for LovenseSetup {
    fn default() -> Self {
        let connection = LovenseConnection::default();
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
    fn connection(&self) -> LovenseConnection {
        LovenseConnection::new(self.domain.trim(), self.http_port)
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProviderSettings {
    pub pishock: PiShockSetup,
    pub openshock: OpenShockSetup,
    pub lovense: LovenseSetup,
}
impl fmt::Debug for ProviderSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderSettings")
            .field("pishock", &self.pishock)
            .field("openshock", &self.openshock)
            .field("lovense", &self.lovense)
            .finish()
    }
}
impl ProviderSettings {
    pub fn setup_present(&self, provider: ProviderKind) -> bool {
        match provider {
            ProviderKind::PiShock => self.pishock.present(),
            ProviderKind::OpenShock => self.openshock.present(),
            ProviderKind::Lovense => self.lovense.present(),
        }
    }
    pub fn connection_config(&self, provider: ProviderKind) -> ProviderConnectionConfig {
        match provider {
            ProviderKind::PiShock => ProviderConnectionConfig::PiShock(self.pishock.normalized()),
            ProviderKind::OpenShock => {
                ProviderConnectionConfig::OpenShock(self.openshock.normalized())
            }
            ProviderKind::Lovense => ProviderConnectionConfig::Lovense(self.lovense.normalized()),
        }
    }
    pub fn normalize(&mut self) {
        self.pishock = self.pishock.normalized();
        self.openshock = self.openshock.normalized();
        self.lovense = self.lovense.normalized();
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ProviderConnectionConfig {
    PiShock(PiShockSetup),
    OpenShock(OpenShockSetup),
    Lovense(LovenseSetup),
}
impl fmt::Debug for ProviderConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PiShock(setup) => f.debug_tuple("PiShock").field(setup).finish(),
            Self::OpenShock(setup) => f.debug_tuple("OpenShock").field(setup).finish(),
            Self::Lovense(setup) => f.debug_tuple("Lovense").field(setup).finish(),
        }
    }
}
impl ProviderConnectionConfig {
    pub fn provider(&self) -> ProviderKind {
        match self {
            Self::PiShock(_) => ProviderKind::PiShock,
            Self::OpenShock(_) => ProviderKind::OpenShock,
            Self::Lovense(_) => ProviderKind::Lovense,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetId {
    PiShock(u64),
    OpenShock(String),
    Lovense(String),
}
#[derive(Clone)]
enum TargetData {
    PiShock(Device),
    OpenShock(DeviceGroup),
    Lovense(Toy),
}
#[derive(Clone)]
pub struct ProviderTarget {
    id: TargetId,
    name: String,
    data: Option<TargetData>,
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
    #[cfg(test)]
    pub(crate) fn new(id: TargetId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            data: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("PiShock: {0}")]
    PiShock(#[from] PiShockError),
    #[error("OpenShock: {0}")]
    OpenShock(#[from] OpenShockError),
    #[error("Lovense: {0}")]
    Lovense(#[from] LovenseError),
    #[error("provider connection configuration does not belong to the selected provider")]
    ConnectionConfigMismatch,
    #[error("provider target does not belong to the active provider")]
    TargetProviderMismatch,
    #[error("resolved action does not belong to the active provider action family")]
    ActionProviderMismatch,
    #[error("provider target is required")]
    TargetRequired,
    #[error("provider setup is invalid")]
    InvalidSetup,
    #[error("no provider is connected")]
    NotConnected,
}

const TEST_VIBRATE_STRENGTH: u8 = 8;
const TEST_VIBRATE_DURATION_SECS: u32 = 1;

pub enum ConnectedProvider {
    PiShock(WebSocketClient),
    OpenShock(OpenShockClient),
    Lovense(LovenseClient),
    #[cfg(test)]
    Test(ProviderKind),
}
impl ConnectedProvider {
    pub fn kind(&self) -> ProviderKind {
        match self {
            Self::PiShock(_) => ProviderKind::PiShock,
            Self::OpenShock(_) => ProviderKind::OpenShock,
            Self::Lovense(_) => ProviderKind::Lovense,
            #[cfg(test)]
            Self::Test(provider) => *provider,
        }
    }
    pub fn connect(
        kind: ProviderKind,
        config: &ProviderConnectionConfig,
    ) -> Result<Self, ProviderError> {
        if config.provider() != kind {
            return Err(ProviderError::ConnectionConfigMismatch);
        }
        match (kind, config) {
            (ProviderKind::PiShock, ProviderConnectionConfig::PiShock(setup))
                if setup.present() =>
            {
                Ok(Self::PiShock(WebSocketClient::connect(
                    PiShockCredentials::new(setup.username.trim(), setup.api_key.trim()),
                    SENDER_NAME,
                )?))
            }
            (ProviderKind::OpenShock, ProviderConnectionConfig::OpenShock(setup))
                if setup.present() =>
            {
                Ok(Self::OpenShock(OpenShockClient::connect(
                    OpenShockCredentials::new(setup.token.trim()),
                    SENDER_NAME,
                )?))
            }
            (ProviderKind::Lovense, ProviderConnectionConfig::Lovense(setup))
                if setup.present() =>
            {
                Ok(Self::Lovense(LovenseClient::connect(setup.connection())?))
            }
            _ => Err(ProviderError::InvalidSetup),
        }
    }
    pub fn list_targets(&self) -> Result<Vec<ProviderTarget>, ProviderError> {
        match self {
            Self::PiShock(client) => Ok(client
                .list_devices()?
                .into_iter()
                .map(target_from_pishock)
                .collect()),
            Self::OpenShock(client) => Ok(client
                .list_devices()?
                .into_iter()
                .map(target_from_openshock)
                .collect()),
            Self::Lovense(client) => Ok(client
                .list_toys()?
                .into_iter()
                .map(target_from_lovense)
                .collect()),
            #[cfg(test)]
            Self::Test(_) => Err(ProviderError::NotConnected),
        }
    }
    pub fn test_action(
        &self,
        action: TestActionKind,
        target: Option<&ProviderTarget>,
    ) -> Result<(), ProviderError> {
        match (self, action, target) {
            (Self::PiShock(_), TestActionKind::Sound, None)
            | (Self::OpenShock(_), TestActionKind::Sound, None) => {
                Err(ProviderError::TargetRequired)
            }
            #[cfg(test)]
            (Self::Test(_), TestActionKind::Sound, None) => Err(ProviderError::TargetRequired),
            (Self::PiShock(client), TestActionKind::Sound, Some(target)) => {
                self.validate_target(target)?;
                match target.data.as_ref() {
                    Some(TargetData::PiShock(device)) => {
                        client.beep_device(device, TEST_SOUND_DURATION.as_secs() as u8)?;
                        Ok(())
                    }
                    Some(TargetData::OpenShock(_) | TargetData::Lovense(_)) => {
                        Err(ProviderError::TargetProviderMismatch)
                    }
                    None => Err(ProviderError::NotConnected),
                }
            }
            (Self::OpenShock(client), TestActionKind::Sound, Some(target)) => {
                self.validate_target(target)?;
                match target.data.as_ref() {
                    Some(TargetData::OpenShock(group)) => {
                        client.test_sound(group, TEST_SOUND_DURATION.as_millis() as u64)?;
                        Ok(())
                    }
                    Some(TargetData::PiShock(_) | TargetData::Lovense(_)) => {
                        Err(ProviderError::TargetProviderMismatch)
                    }
                    None => Err(ProviderError::NotConnected),
                }
            }
            (Self::Lovense(client), TestActionKind::Pulse, target) => {
                let toy = match target {
                    Some(target) => {
                        self.validate_target(target)?;
                        match target.data.as_ref() {
                            Some(TargetData::Lovense(toy)) => Some(toy),
                            Some(TargetData::PiShock(_) | TargetData::OpenShock(_)) => {
                                return Err(ProviderError::TargetProviderMismatch);
                            }
                            None => return Err(ProviderError::NotConnected),
                        }
                    }
                    None => None,
                };
                client.vibrate(toy, TEST_VIBRATE_STRENGTH, TEST_VIBRATE_DURATION_SECS)?;
                Ok(())
            }
            #[cfg(test)]
            (Self::Test(provider), TestActionKind::Sound | TestActionKind::Pulse, Some(target)) => {
                if target_matches(*provider, target.id()) {
                    Err(ProviderError::NotConnected)
                } else {
                    Err(ProviderError::TargetProviderMismatch)
                }
            }
            _ => Err(ProviderError::ActionProviderMismatch),
        }
    }
    pub fn execute(
        &self,
        target: Option<&ProviderTarget>,
        action: ResolvedAction,
    ) -> Result<(), ProviderError> {
        match (self, target, action) {
            (Self::PiShock(_), None, ResolvedAction::Shock(_))
            | (Self::OpenShock(_), None, ResolvedAction::Shock(_)) => {
                Err(ProviderError::TargetRequired)
            }
            #[cfg(test)]
            (Self::Test(_), None, ResolvedAction::Shock(_)) => Err(ProviderError::TargetRequired),
            (Self::PiShock(client), Some(target), ResolvedAction::Shock(shock)) => {
                self.validate_target(target)?;
                match target.data.as_ref() {
                    Some(TargetData::PiShock(device)) => {
                        client.shock_device(device, shock.intensity, shock.duration_ms)?;
                        Ok(())
                    }
                    Some(TargetData::OpenShock(_) | TargetData::Lovense(_)) => {
                        Err(ProviderError::TargetProviderMismatch)
                    }
                    None => Err(ProviderError::NotConnected),
                }
            }
            (Self::OpenShock(client), Some(target), ResolvedAction::Shock(shock)) => {
                self.validate_target(target)?;
                match target.data.as_ref() {
                    Some(TargetData::OpenShock(group)) => {
                        client.shock(group, shock.intensity, shock.duration_ms)?;
                        Ok(())
                    }
                    Some(TargetData::PiShock(_) | TargetData::Lovense(_)) => {
                        Err(ProviderError::TargetProviderMismatch)
                    }
                    None => Err(ProviderError::NotConnected),
                }
            }
            (Self::Lovense(client), target, ResolvedAction::Vibrate(vibrate)) => {
                let toy = match target {
                    Some(target) => {
                        self.validate_target(target)?;
                        match target.data.as_ref() {
                            Some(TargetData::Lovense(toy)) => Some(toy),
                            Some(TargetData::PiShock(_) | TargetData::OpenShock(_)) => {
                                return Err(ProviderError::TargetProviderMismatch);
                            }
                            None => return Err(ProviderError::NotConnected),
                        }
                    }
                    None => None,
                };
                client.vibrate(toy, vibrate.strength, vibrate.duration_secs)?;
                Ok(())
            }
            #[cfg(test)]
            (Self::Test(provider), Some(target), ResolvedAction::Shock(_)) => {
                if target_matches(*provider, target.id()) {
                    Err(ProviderError::NotConnected)
                } else {
                    Err(ProviderError::TargetProviderMismatch)
                }
            }
            _ => Err(ProviderError::ActionProviderMismatch),
        }
    }
    fn validate_target(&self, target: &ProviderTarget) -> Result<(), ProviderError> {
        if target_matches(self.kind(), target.id()) {
            Ok(())
        } else {
            Err(ProviderError::TargetProviderMismatch)
        }
    }
    pub fn disconnect(self) -> Result<(), ProviderError> {
        match self {
            Self::PiShock(_) => Ok(()),
            Self::OpenShock(client) => Ok(client.disconnect()?),
            Self::Lovense(_) => Ok(()),
            #[cfg(test)]
            Self::Test(_) => Ok(()),
        }
    }
}
fn target_matches(provider: ProviderKind, target: &TargetId) -> bool {
    matches!(
        (provider, target),
        (ProviderKind::PiShock, TargetId::PiShock(_))
            | (ProviderKind::OpenShock, TargetId::OpenShock(_))
            | (ProviderKind::Lovense, TargetId::Lovense(_))
    )
}
fn target_from_pishock(device: Device) -> ProviderTarget {
    ProviderTarget {
        id: TargetId::PiShock(device.client_id),
        name: device.name.clone(),
        data: Some(TargetData::PiShock(device)),
    }
}
fn target_from_openshock(group: DeviceGroup) -> ProviderTarget {
    ProviderTarget {
        id: TargetId::OpenShock(group.id.clone()),
        name: group.name.clone(),
        data: Some(TargetData::OpenShock(group)),
    }
}
fn target_from_lovense(toy: Toy) -> ProviderTarget {
    ProviderTarget {
        id: TargetId::Lovense(toy.id.clone()),
        name: toy.name.clone(),
        data: Some(TargetData::Lovense(toy)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_provider_is_pishock() {
        assert_eq!(ProviderKind::default(), ProviderKind::PiShock);
    }
    #[test]
    fn setup_debug_redacts_secrets() {
        const PISHOCK_SECRET: &str = "pishock-secret-value";
        const OPENSHOCK_SECRET: &str = "openshock-secret-value";
        let setup = ProviderSettings {
            pishock: PiShockSetup {
                username: "u".into(),
                api_key: PISHOCK_SECRET.into(),
            },
            openshock: OpenShockSetup {
                token: OPENSHOCK_SECRET.into(),
            },
            lovense: LovenseSetup::default(),
        };
        let representations = [
            format!("{setup:?}"),
            format!("{:?}", setup.connection_config(ProviderKind::PiShock)),
            format!("{:?}", setup.connection_config(ProviderKind::OpenShock)),
        ];
        for representation in representations {
            assert!(!representation.contains(PISHOCK_SECRET));
            assert!(!representation.contains(OPENSHOCK_SECRET));
        }
    }
    #[test]
    fn descriptors_are_exhaustive_for_current_providers() {
        assert_eq!(ProviderKind::PiShock.action_kind(), ActionKind::Shock);
        assert_eq!(ProviderKind::OpenShock.action_kind(), ActionKind::Shock);
        assert_eq!(ProviderKind::Lovense.action_kind(), ActionKind::Vibrate);
    }
    #[test]
    fn target_policies_are_explicit_and_bounded() {
        assert!(TargetPolicy::Required.accepts(true));
        assert!(!TargetPolicy::Required.accepts(false));
        assert!(TargetPolicy::Optional.accepts(false));
        assert!(TargetPolicy::None.accepts(false));
    }
    #[test]
    fn tagged_target_matching_is_exhaustive() {
        assert!(target_matches(ProviderKind::PiShock, &TargetId::PiShock(1)));
        assert!(!target_matches(
            ProviderKind::PiShock,
            &TargetId::OpenShock("x".into())
        ));
        assert!(target_matches(
            ProviderKind::OpenShock,
            &TargetId::OpenShock("x".into())
        ));
        assert!(!target_matches(
            ProviderKind::OpenShock,
            &TargetId::PiShock(1)
        ));
    }
    #[test]
    fn descriptors_declare_test_action_capability() {
        for provider in [ProviderKind::PiShock, ProviderKind::OpenShock] {
            assert_eq!(
                provider.descriptor().test_action,
                Some(TestActionKind::Sound)
            );
            assert_eq!(
                provider.descriptor().test_action.map(TestActionKind::label),
                Some("Send test sound")
            );
        }
        assert_eq!(
            ProviderKind::Lovense.descriptor().test_action,
            Some(TestActionKind::Pulse)
        );
        assert_eq!(
            ProviderKind::Lovense
                .descriptor()
                .test_action
                .map(TestActionKind::label),
            Some("Send test vibration")
        );
    }

    #[test]
    fn required_adapter_rejects_targetless_execution_before_dispatch() {
        let client = ConnectedProvider::Test(ProviderKind::PiShock);
        let action = ResolvedAction::Shock(crate::action::ResolvedShockAction {
            intensity: 20,
            duration_ms: 500,
        });
        assert_eq!(
            client.execute(None, action).unwrap_err().to_string(),
            "provider target is required"
        );
    }

    #[test]
    fn adapter_rejects_mismatched_targets_before_library_calls() {
        let pishock = ConnectedProvider::Test(ProviderKind::PiShock);
        let openshock_target = ProviderTarget::new(TargetId::OpenShock("group".into()), "group");
        let action = ResolvedAction::Shock(crate::action::ResolvedShockAction {
            intensity: 20,
            duration_ms: 500,
        });
        assert!(matches!(
            pishock.execute(Some(&openshock_target), action),
            Err(ProviderError::TargetProviderMismatch)
        ));
        assert!(matches!(
            pishock.test_action(TestActionKind::Sound, Some(&openshock_target)),
            Err(ProviderError::TargetProviderMismatch)
        ));
    }
}
