use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::action::{
    ActionSettings, MAX_SHOCK_DURATION, MAX_SHOCK_INTENSITY, MIN_SHOCK_DURATION,
    MIN_SHOCK_INTENSITY, ShockActionSettings, ShockFixedSettings, ShockIntervalSettings, ShockMode,
};
use crate::app::{
    AbilityFilter, AbilityTriggerSettings, AppState, TriggerSettings, TriggerSettingsSet,
};
use crate::provider::{
    LovenseSetup, OpenShockSetup, PiShockSetup, ProviderKind, ProviderSettings, TargetId,
};

pub const SCHEMA_VERSION: u32 = 6;
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PersistedState {
    schema_version: u32,
    provider: PersistedProvider,
    provider_settings: PersistedProviderSettings,
    preferred_target: Option<PersistedTarget>,
    triggers: PersistedTriggers,
    log_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedStateV5 {
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    provider: PersistedProvider,
    provider_settings: PersistedProviderSettings,
    preferred_target: Option<PersistedTarget>,
    triggers: PersistedTriggersV5,
    log_path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedStateV4 {
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    provider: PersistedProvider,
    provider_settings: PersistedProviderSettingsV4,
    preferred_target: Option<PersistedTarget>,
    triggers: PersistedTriggersV5,
    log_path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedProviderSettingsV4 {
    pishock: PersistedPiShockSetup,
    openshock: PersistedOpenShockSetup,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedStateV3 {
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    provider: PersistedProvider,
    credentials: PersistedProviderSettingsV4,
    preferred_target: Option<PersistedTarget>,
    triggers: PersistedTriggersV3,
    log_path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedStateV2 {
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    provider: PersistedProvider,
    credentials: PersistedProviderSettingsV4,
    preferred_target: Option<PersistedTarget>,
    shock: PersistedShock,
    triggers: PersistedTriggersV2,
    log_path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedStateV1 {
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    provider: PersistedProvider,
    credentials: PersistedProviderSettingsV4,
    preferred_target: Option<PersistedTarget>,
    shock: PersistedShock,
    log_path: String,
}
#[derive(Deserialize)]
struct SchemaVersion {
    schema_version: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedTriggers {
    local_player_death: PersistedTrigger,
    local_player_kill: PersistedTrigger,
    local_player_assist: PersistedTrigger,
    ability_used: PersistedAbilityTrigger,
    ability_cooldown_ready: PersistedAbilityTrigger,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedTriggersV5 {
    local_player_death: PersistedTrigger,
    ability_used: PersistedAbilityTrigger,
    ability_cooldown_ready: PersistedAbilityTrigger,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedTriggersV3 {
    local_player_death: PersistedTriggerV3,
    ability_used: PersistedAbilityTriggerV3,
    ability_cooldown_ready: PersistedAbilityTriggerV3,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedTriggerV3 {
    enabled: bool,
    shock: PersistedShock,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedAbilityTriggerV3 {
    trigger: PersistedTriggerV3,
    ability_filter: PersistedAbilityFilter,
}
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedTriggersV2 {
    local_player_death: bool,
    ability_used: bool,
    ability_cooldown_ready: bool,
}
impl Default for PersistedTriggersV2 {
    fn default() -> Self {
        Self {
            local_player_death: true,
            ability_used: false,
            ability_cooldown_ready: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedTrigger {
    enabled: bool,
    actions: PersistedActions,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedActions {
    shock: PersistedShock,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedAbilityTrigger {
    trigger: PersistedTrigger,
    ability_filter: PersistedAbilityFilter,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedAbilityFilter {
    mode: PersistedAbilityFilterMode,
    slots: Vec<u32>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PersistedAbilityFilterMode {
    All,
    Selected,
}
impl Default for PersistedAbilityFilter {
    fn default() -> Self {
        Self {
            mode: PersistedAbilityFilterMode::All,
            slots: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PersistedProvider {
    PiShock,
    OpenShock,
    Lovense,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedProviderSettings {
    pishock: PersistedPiShockSetup,
    openshock: PersistedOpenShockSetup,
    lovense: PersistedLovenseSetup,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedPiShockSetup {
    username: String,
    api_key: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedOpenShockSetup {
    token: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedLovenseSetup {
    domain: String,
    http_port: u16,
}
impl Default for PersistedLovenseSetup {
    fn default() -> Self {
        let setup = LovenseSetup::default();
        Self {
            domain: setup.domain,
            http_port: setup.http_port,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedTarget {
    provider: PersistedProvider,
    id: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedShock {
    mode: PersistedShockMode,
    interval: PersistedShockInterval,
    fixed: PersistedShockFixed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PersistedShockMode {
    Interval,
    Fixed,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedShockInterval {
    minimum_intensity: f32,
    maximum_intensity: f32,
    minimum_duration_seconds: f32,
    maximum_duration_seconds: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedShockFixed {
    intensity: f32,
    duration_seconds: f32,
}
impl Default for PersistedShock {
    fn default() -> Self {
        Self {
            mode: PersistedShockMode::Interval,
            interval: PersistedShockInterval {
                minimum_intensity: MIN_SHOCK_INTENSITY,
                maximum_intensity: MIN_SHOCK_INTENSITY,
                minimum_duration_seconds: MIN_SHOCK_DURATION,
                maximum_duration_seconds: MIN_SHOCK_DURATION,
            },
            fixed: PersistedShockFixed {
                intensity: MIN_SHOCK_INTENSITY,
                duration_seconds: MIN_SHOCK_DURATION,
            },
        }
    }
}
fn disabled_trigger(shock: PersistedShock) -> PersistedTrigger {
    PersistedTrigger {
        enabled: false,
        actions: PersistedActions { shock },
    }
}
impl Default for PersistedTriggers {
    fn default() -> Self {
        let shock = PersistedShock::default();
        Self {
            local_player_death: PersistedTrigger {
                enabled: true,
                actions: PersistedActions {
                    shock: shock.clone(),
                },
            },
            local_player_kill: disabled_trigger(shock.clone()),
            local_player_assist: disabled_trigger(shock.clone()),
            ability_used: PersistedAbilityTrigger {
                trigger: PersistedTrigger {
                    enabled: false,
                    actions: PersistedActions {
                        shock: shock.clone(),
                    },
                },
                ability_filter: PersistedAbilityFilter::default(),
            },
            ability_cooldown_ready: PersistedAbilityTrigger {
                trigger: PersistedTrigger {
                    enabled: false,
                    actions: PersistedActions {
                        shock: shock.clone(),
                    },
                },
                ability_filter: PersistedAbilityFilter::default(),
            },
        }
    }
}
impl Default for PersistedProviderSettings {
    fn default() -> Self {
        Self {
            pishock: PersistedPiShockSetup {
                username: String::new(),
                api_key: String::new(),
            },
            openshock: PersistedOpenShockSetup {
                token: String::new(),
            },
            lovense: PersistedLovenseSetup::default(),
        }
    }
}
impl From<PersistedProviderSettingsV4> for PersistedProviderSettings {
    fn from(settings: PersistedProviderSettingsV4) -> Self {
        Self {
            pishock: settings.pishock,
            openshock: settings.openshock,
            lovense: PersistedLovenseSetup::default(),
        }
    }
}
impl Default for PersistedState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            provider: PersistedProvider::PiShock,
            provider_settings: PersistedProviderSettings::default(),
            preferred_target: None,
            triggers: PersistedTriggers::default(),
            log_path: String::new(),
        }
    }
}
impl PersistedTriggers {
    fn from_shared(shock: PersistedShock, enabled: PersistedTriggersV2) -> Self {
        Self {
            local_player_death: PersistedTrigger {
                enabled: enabled.local_player_death,
                actions: PersistedActions {
                    shock: shock.clone(),
                },
            },
            local_player_kill: disabled_trigger(shock.clone()),
            local_player_assist: disabled_trigger(shock.clone()),
            ability_used: PersistedAbilityTrigger {
                trigger: PersistedTrigger {
                    enabled: enabled.ability_used,
                    actions: PersistedActions {
                        shock: shock.clone(),
                    },
                },
                ability_filter: PersistedAbilityFilter::default(),
            },
            ability_cooldown_ready: PersistedAbilityTrigger {
                trigger: PersistedTrigger {
                    enabled: enabled.ability_cooldown_ready,
                    actions: PersistedActions { shock },
                },
                ability_filter: PersistedAbilityFilter::default(),
            },
        }
    }
    fn from_v3(source: PersistedTriggersV3) -> Self {
        Self {
            local_player_death: PersistedTrigger {
                enabled: source.local_player_death.enabled,
                actions: PersistedActions {
                    shock: source.local_player_death.shock.clone(),
                },
            },
            local_player_kill: disabled_trigger(source.local_player_death.shock.clone()),
            local_player_assist: disabled_trigger(source.local_player_death.shock),
            ability_used: PersistedAbilityTrigger {
                trigger: PersistedTrigger {
                    enabled: source.ability_used.trigger.enabled,
                    actions: PersistedActions {
                        shock: source.ability_used.trigger.shock,
                    },
                },
                ability_filter: source.ability_used.ability_filter,
            },
            ability_cooldown_ready: PersistedAbilityTrigger {
                trigger: PersistedTrigger {
                    enabled: source.ability_cooldown_ready.trigger.enabled,
                    actions: PersistedActions {
                        shock: source.ability_cooldown_ready.trigger.shock,
                    },
                },
                ability_filter: source.ability_cooldown_ready.ability_filter,
            },
        }
    }
    fn from_v5(source: PersistedTriggersV5) -> Self {
        Self {
            local_player_death: source.local_player_death.clone(),
            local_player_kill: disabled_trigger(source.local_player_death.actions.shock.clone()),
            local_player_assist: disabled_trigger(source.local_player_death.actions.shock),
            ability_used: source.ability_used,
            ability_cooldown_ready: source.ability_cooldown_ready,
        }
    }
}
impl From<PersistedStateV1> for PersistedState {
    fn from(state: PersistedStateV1) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            provider: state.provider,
            provider_settings: state.credentials.into(),
            preferred_target: state.preferred_target,
            triggers: PersistedTriggers::from_shared(state.shock, PersistedTriggersV2::default()),
            log_path: state.log_path,
        }
    }
}
impl From<PersistedStateV2> for PersistedState {
    fn from(state: PersistedStateV2) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            provider: state.provider,
            provider_settings: state.credentials.into(),
            preferred_target: state.preferred_target,
            triggers: PersistedTriggers::from_shared(state.shock, state.triggers),
            log_path: state.log_path,
        }
    }
}
impl From<PersistedStateV4> for PersistedState {
    fn from(state: PersistedStateV4) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            provider: state.provider,
            provider_settings: state.provider_settings.into(),
            preferred_target: state.preferred_target,
            triggers: PersistedTriggers::from_v5(state.triggers),
            log_path: state.log_path,
        }
    }
}
impl From<PersistedStateV5> for PersistedState {
    fn from(state: PersistedStateV5) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            provider: state.provider,
            provider_settings: state.provider_settings,
            preferred_target: state.preferred_target,
            triggers: PersistedTriggers::from_v5(state.triggers),
            log_path: state.log_path,
        }
    }
}
impl From<PersistedStateV3> for PersistedState {
    fn from(state: PersistedStateV3) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            provider: state.provider,
            provider_settings: state.credentials.into(),
            preferred_target: state.preferred_target,
            triggers: PersistedTriggers::from_v3(state.triggers),
            log_path: state.log_path,
        }
    }
}

impl PersistedState {
    pub(crate) fn from_app(app: &AppState) -> Self {
        let setup = app.effective_provider_settings();
        let state = Self {
            schema_version: SCHEMA_VERSION,
            provider: app.provider.into(),
            provider_settings: PersistedProviderSettings {
                pishock: PersistedPiShockSetup {
                    username: setup.pishock.username,
                    api_key: setup.pishock.api_key,
                },
                openshock: PersistedOpenShockSetup {
                    token: setup.openshock.token,
                },
                lovense: PersistedLovenseSetup {
                    domain: setup.lovense.domain,
                    http_port: setup.lovense.http_port,
                },
            },
            preferred_target: app
                .preferred_target
                .as_ref()
                .map(PersistedTarget::from_target_id),
            triggers: PersistedTriggers::from_app(&app.triggers),
            log_path: app.log_path.clone(),
        };
        state.normalized().unwrap_or_default()
    }
    pub(crate) fn restore_app(&self) -> AppState {
        let mut app = AppState::default();
        app.provider = self.provider.into();
        app.provider_settings = ProviderSettings {
            pishock: PiShockSetup {
                username: self.provider_settings.pishock.username.clone(),
                api_key: self.provider_settings.pishock.api_key.clone(),
            },
            openshock: OpenShockSetup {
                token: self.provider_settings.openshock.token.clone(),
            },
            lovense: LovenseSetup {
                domain: self.provider_settings.lovense.domain.clone(),
                http_port: self.provider_settings.lovense.http_port,
            },
        };
        app.preferred_target = self
            .preferred_target
            .as_ref()
            .and_then(|target| target.to_target_id().ok());
        app.triggers = self.triggers.to_app();
        app.log_path = self.log_path.clone();
        app
    }
    fn normalized(mut self) -> Result<Self, String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported schema version {}; expected {SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        self.triggers.local_player_death.actions.shock.normalize();
        self.triggers.local_player_kill.actions.shock.normalize();
        self.triggers.local_player_assist.actions.shock.normalize();
        self.triggers.ability_used.trigger.actions.shock.normalize();
        self.triggers
            .ability_cooldown_ready
            .trigger
            .actions
            .shock
            .normalize();
        self.triggers.ability_used.ability_filter.normalize();
        self.triggers
            .ability_cooldown_ready
            .ability_filter
            .normalize();
        if let Some(target) = &self.preferred_target {
            let canonical = PersistedTarget::from_target_id(&target.to_target_id()?);
            self.preferred_target = (canonical.provider == self.provider).then_some(canonical);
        }
        Ok(self)
    }
}
impl PersistedTriggers {
    fn from_app(triggers: &TriggerSettingsSet) -> Self {
        Self {
            local_player_death: PersistedTrigger::from_app(&triggers.death),
            local_player_kill: PersistedTrigger::from_app(&triggers.kill),
            local_player_assist: PersistedTrigger::from_app(&triggers.assist),
            ability_used: PersistedAbilityTrigger::from_app(&triggers.ability_use),
            ability_cooldown_ready: PersistedAbilityTrigger::from_app(
                &triggers.ability_cooldown_ready,
            ),
        }
    }
    fn to_app(&self) -> TriggerSettingsSet {
        TriggerSettingsSet {
            death: self.local_player_death.to_app(),
            kill: self.local_player_kill.to_app(),
            assist: self.local_player_assist.to_app(),
            ability_use: self.ability_used.to_app(),
            ability_cooldown_ready: self.ability_cooldown_ready.to_app(),
        }
    }
}
impl PersistedTrigger {
    fn from_app(trigger: &TriggerSettings) -> Self {
        Self {
            enabled: trigger.enabled,
            actions: PersistedActions {
                shock: PersistedShock::from_app(&trigger.actions.shock),
            },
        }
    }
    fn to_app(&self) -> TriggerSettings {
        TriggerSettings {
            enabled: self.enabled,
            actions: ActionSettings {
                shock: self.actions.shock.to_app(),
                // Vibrate settings are not persisted yet; they reset to
                // their defaults on restart until PersistedActions gains a
                // versioned `vibrate` field.
                vibrate: crate::action::VibrateActionSettings::default(),
            },
        }
    }
}
impl PersistedAbilityTrigger {
    fn from_app(trigger: &AbilityTriggerSettings) -> Self {
        Self {
            trigger: PersistedTrigger::from_app(&trigger.trigger),
            ability_filter: PersistedAbilityFilter::from_app(&trigger.ability_filter),
        }
    }
    fn to_app(&self) -> AbilityTriggerSettings {
        AbilityTriggerSettings {
            trigger: self.trigger.to_app(),
            ability_filter: self.ability_filter.to_app(),
        }
    }
}
impl PersistedAbilityFilter {
    fn from_app(filter: &AbilityFilter) -> Self {
        match filter {
            AbilityFilter::All => Self::default(),
            AbilityFilter::Selected(slots) => Self {
                mode: PersistedAbilityFilterMode::Selected,
                slots: slots.iter().copied().filter(|slot| *slot > 0).collect(),
            },
        }
    }
    fn to_app(&self) -> AbilityFilter {
        match self.mode {
            PersistedAbilityFilterMode::All => AbilityFilter::All,
            PersistedAbilityFilterMode::Selected => AbilityFilter::Selected(
                self.slots
                    .iter()
                    .copied()
                    .filter(|slot| *slot > 0)
                    .collect::<BTreeSet<_>>(),
            ),
        }
    }
    fn normalize(&mut self) {
        if self.mode == PersistedAbilityFilterMode::All {
            self.slots.clear();
        } else {
            self.slots.retain(|slot| *slot > 0);
            self.slots.sort_unstable();
            self.slots.dedup();
        }
    }
}
impl PersistedShock {
    fn from_app(shock: &ShockActionSettings) -> Self {
        Self {
            mode: shock.mode.into(),
            interval: PersistedShockInterval {
                minimum_intensity: shock.interval.minimum_intensity,
                maximum_intensity: shock.interval.maximum_intensity,
                minimum_duration_seconds: shock.interval.minimum_duration_seconds,
                maximum_duration_seconds: shock.interval.maximum_duration_seconds,
            },
            fixed: PersistedShockFixed {
                intensity: shock.fixed.intensity,
                duration_seconds: shock.fixed.duration_seconds,
            },
        }
    }
    fn to_app(&self) -> ShockActionSettings {
        ShockActionSettings {
            mode: self.mode.into(),
            interval: ShockIntervalSettings {
                minimum_intensity: self.interval.minimum_intensity,
                maximum_intensity: self.interval.maximum_intensity,
                minimum_duration_seconds: self.interval.minimum_duration_seconds,
                maximum_duration_seconds: self.interval.maximum_duration_seconds,
            },
            fixed: ShockFixedSettings {
                intensity: self.fixed.intensity,
                duration_seconds: self.fixed.duration_seconds,
            },
        }
    }
    fn normalize(&mut self) {
        self.interval.minimum_intensity = normalize_value(
            self.interval.minimum_intensity,
            MIN_SHOCK_INTENSITY,
            MAX_SHOCK_INTENSITY,
            MIN_SHOCK_INTENSITY,
        );
        self.interval.maximum_intensity = normalize_value(
            self.interval.maximum_intensity,
            MIN_SHOCK_INTENSITY,
            MAX_SHOCK_INTENSITY,
            MIN_SHOCK_INTENSITY,
        )
        .max(self.interval.minimum_intensity);
        self.fixed.intensity = normalize_value(
            self.fixed.intensity,
            MIN_SHOCK_INTENSITY,
            MAX_SHOCK_INTENSITY,
            MIN_SHOCK_INTENSITY,
        );
        self.interval.minimum_duration_seconds = normalize_value(
            self.interval.minimum_duration_seconds,
            MIN_SHOCK_DURATION,
            MAX_SHOCK_DURATION,
            MIN_SHOCK_DURATION,
        );
        self.interval.maximum_duration_seconds = normalize_value(
            self.interval.maximum_duration_seconds,
            MIN_SHOCK_DURATION,
            MAX_SHOCK_DURATION,
            MIN_SHOCK_DURATION,
        )
        .max(self.interval.minimum_duration_seconds);
        self.fixed.duration_seconds = normalize_value(
            self.fixed.duration_seconds,
            MIN_SHOCK_DURATION,
            MAX_SHOCK_DURATION,
            MIN_SHOCK_DURATION,
        );
    }
}
impl From<ProviderKind> for PersistedProvider {
    fn from(provider: ProviderKind) -> Self {
        match provider {
            ProviderKind::PiShock => Self::PiShock,
            ProviderKind::OpenShock => Self::OpenShock,
            ProviderKind::Lovense => Self::Lovense,
        }
    }
}
impl From<PersistedProvider> for ProviderKind {
    fn from(provider: PersistedProvider) -> Self {
        match provider {
            PersistedProvider::PiShock => Self::PiShock,
            PersistedProvider::OpenShock => Self::OpenShock,
            PersistedProvider::Lovense => Self::Lovense,
        }
    }
}
impl From<ShockMode> for PersistedShockMode {
    fn from(mode: ShockMode) -> Self {
        match mode {
            ShockMode::Interval => Self::Interval,
            ShockMode::Fixed => Self::Fixed,
        }
    }
}
impl From<PersistedShockMode> for ShockMode {
    fn from(mode: PersistedShockMode) -> Self {
        match mode {
            PersistedShockMode::Interval => Self::Interval,
            PersistedShockMode::Fixed => Self::Fixed,
        }
    }
}
impl PersistedTarget {
    fn from_target_id(target: &TargetId) -> Self {
        match target {
            TargetId::PiShock(id) => Self {
                provider: PersistedProvider::PiShock,
                id: id.to_string(),
            },
            TargetId::OpenShock(id) => Self {
                provider: PersistedProvider::OpenShock,
                id: id.clone(),
            },
            TargetId::Lovense(id) => Self {
                provider: PersistedProvider::Lovense,
                id: id.clone(),
            },
        }
    }
    fn to_target_id(&self) -> Result<TargetId, String> {
        match self.provider {
            PersistedProvider::PiShock => {
                self.id.parse::<u64>().map(TargetId::PiShock).map_err(|_| {
                    "PiShock preferred target ID is not an unsigned integer".to_owned()
                })
            }
            PersistedProvider::OpenShock if self.id.trim().is_empty() => {
                Err("OpenShock preferred target ID is empty".to_owned())
            }
            PersistedProvider::OpenShock => Ok(TargetId::OpenShock(self.id.clone())),
            PersistedProvider::Lovense if self.id.trim().is_empty() => {
                Err("Lovense preferred target ID is empty".to_owned())
            }
            PersistedProvider::Lovense => Ok(TargetId::Lovense(self.id.clone())),
        }
    }
}

fn normalize_value(value: f32, minimum: f32, maximum: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(minimum, maximum)
    } else {
        fallback
    }
}

pub(crate) struct LoadOutcome {
    pub state: PersistedState,
    pub warning: Option<String>,
    migrated: bool,
}

pub(crate) fn default_state_path() -> Result<PathBuf, String> {
    let result = dirs::config_dir()
        .map(|directory| directory.join("deadlockshock-companion").join("state.json"))
        .ok_or_else(|| {
            "The operating system did not provide a per-user config directory.".to_owned()
        });
    if let Ok(path) = &result {
        log::info!(
            target: "companion::persistence",
            "settings_path_resolved path={:?}",
            path
        );
    }
    result
}

pub(crate) fn load_from_path(path: &Path) -> LoadOutcome {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            log::info!(
                target: "companion::persistence",
                "settings_load_outcome path={:?} outcome=missing_defaults",
                path
            );
            return LoadOutcome {
                state: PersistedState::default(),
                warning: None,
                migrated: false,
            };
        }
        Err(error) => {
            log::warn!(
                target: "companion::persistence",
                "settings_load_failed path={:?} stage=read error={:?}",
                path,
                error
            );
            return LoadOutcome {
                state: PersistedState::default(),
                warning: Some(format!(
                    "Could not read saved state at {}: {error}. Defaults were restored.",
                    path.display()
                )),
                migrated: false,
            };
        }
    };

    let loaded = serde_json::from_str::<SchemaVersion>(&source)
        .map_err(|error| error.to_string())
        .and_then(|version| match version.schema_version {
            1 => serde_json::from_str::<PersistedStateV1>(&source)
                .map(|state| (PersistedState::from(state), true))
                .map_err(|error| error.to_string()),
            2 => serde_json::from_str::<PersistedStateV2>(&source)
                .map(|state| (PersistedState::from(state), true))
                .map_err(|error| error.to_string()),
            3 => serde_json::from_str::<PersistedStateV3>(&source)
                .map(|state| (PersistedState::from(state), true))
                .map_err(|error| error.to_string()),
            4 => serde_json::from_str::<PersistedStateV4>(&source)
                .map(|state| (PersistedState::from(state), true))
                .map_err(|error| error.to_string()),
            5 => serde_json::from_str::<PersistedStateV5>(&source)
                .map(|state| (PersistedState::from(state), true))
                .map_err(|error| error.to_string()),
            SCHEMA_VERSION => serde_json::from_str::<PersistedState>(&source)
                .map(|state| (state, false))
                .map_err(|error| error.to_string()),
            unsupported => Err(format!(
                "unsupported schema version {unsupported}; expected 1, 2, 3, 4, 5, or {SCHEMA_VERSION}"
            )),
        })
        .and_then(|(state, migrated)| state.normalized().map(|state| (state, migrated)));
    match loaded {
        Ok((state, migrated)) => {
            log::info!(
                target: "companion::persistence",
                "settings_load_outcome path={:?} outcome=loaded migrated={}",
                path,
                migrated
            );
            LoadOutcome {
                state,
                warning: None,
                migrated,
            }
        }
        Err(error) => {
            let preservation = match preserve_invalid_file(path) {
                Ok(backup) => {
                    log::warn!(
                        target: "companion::persistence",
                        "settings_load_failed path={:?} stage=parse backup={:?}",
                        path,
                        backup
                    );
                    format!("The invalid file was preserved at {}.", backup.display())
                }
                Err(backup_error) => {
                    log::warn!(
                        target: "companion::persistence",
                        "settings_load_failed path={:?} stage=parse backup_failed error={:?}",
                        path,
                        backup_error
                    );
                    format!(
                        "The invalid file could not be moved to a backup ({backup_error}); it remains at {}.",
                        path.display()
                    )
                }
            };
            LoadOutcome {
                state: PersistedState::default(),
                warning: Some(format!(
                    "Saved state was invalid ({error}). {preservation} Defaults were restored."
                )),
                migrated: false,
            }
        }
    }
}

fn preserve_invalid_file(path: &Path) -> io::Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("state");
    let extension = path.extension().and_then(|extension| extension.to_str());
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));

    for collision in 0_u32.. {
        let suffix = if collision == 0 {
            String::new()
        } else {
            format!("-{collision}")
        };
        let mut file_name = format!(
            "{stem}.invalid-{}-{:09}{suffix}",
            timestamp.as_secs(),
            timestamp.subsec_nanos()
        );
        if let Some(extension) = extension {
            file_name.push('.');
            file_name.push_str(extension);
        }
        let backup = parent.join(file_name);
        if !backup.exists() {
            fs::rename(path, &backup)?;
            return Ok(backup);
        }
    }
    unreachable!("the invalid-state backup suffix space is inexhaustible")
}

fn write_state(path: &Path, state: &PersistedState) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Could not create the saved-state directory {}: {error}",
            parent.display()
        )
    })?;
    set_private_directory_permissions(parent)?;

    let mut temporary = NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "Could not create a temporary saved-state file in {}: {error}",
            parent.display()
        )
    })?;
    set_private_file_permissions(temporary.as_file())?;
    serde_json::to_writer_pretty(&mut temporary, state)
        .map_err(|error| format!("Could not serialize saved state: {error}"))?;
    temporary
        .write_all(b"\n")
        .map_err(|error| format!("Could not finish writing saved state: {error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("Could not synchronize saved state: {error}"))?;
    temporary.persist(path).map_err(|error| {
        format!(
            "Could not atomically replace {}: {}",
            path.display(),
            error.error
        )
    })?;
    Ok(())
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
        format!(
            "Could not restrict saved-state directory permissions for {}: {error}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(file: &fs::File) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("Could not restrict saved-state file permissions: {error}"))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_file: &fs::File) -> Result<(), String> {
    Ok(())
}
#[cfg(target_os = "windows")]
fn open_directory(path: &Path) -> Result<(), String> {
    spawn_directory_opener("explorer", path)
}

#[cfg(any(target_os = "windows", test))]
fn spawn_directory_opener(program: &str, path: &Path) -> Result<(), String> {
    // Explorer commonly exits with code 1 after successfully handing the folder
    // off to the existing shell process, so only failure to launch is an error.
    Command::new(program)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            format!(
                "Could not start the operating system's folder opener for {}: {error}",
                path.display()
            )
        })
}

#[cfg(target_os = "macos")]
fn open_directory(path: &Path) -> Result<(), String> {
    run_directory_opener("open", path)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_directory(path: &Path) -> Result<(), String> {
    run_directory_opener("xdg-open", path)
}

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
fn open_directory(_path: &Path) -> Result<(), String> {
    Err("Opening the config folder is unsupported on this operating system.".to_owned())
}

#[cfg(unix)]
fn run_directory_opener(program: &str, path: &Path) -> Result<(), String> {
    let status = Command::new(program).arg(path).status().map_err(|error| {
        format!(
            "Could not start the operating system's folder opener for {}: {error}",
            path.display()
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "The operating system's folder opener failed for {} with status {status}.",
            path.display()
        ))
    }
}

#[derive(Clone, Copy)]
enum SaveReason {
    Autosave,
    Reset,
}

pub(crate) struct Persistence {
    path: Option<PathBuf>,
    saved: PersistedState,
    observed: PersistedState,
    pending: Option<PersistedState>,
    pending_reason: SaveReason,
    deadline: Option<Instant>,
    debounce: Duration,
    load_warning: Option<String>,
    save_error: Option<String>,
}

impl Persistence {
    pub(crate) fn open(path: PathBuf) -> (Self, PersistedState) {
        let LoadOutcome {
            state,
            warning,
            migrated,
        } = load_from_path(&path);
        log::info!(
            target: "companion::persistence",
            "settings_opened path={:?} load_warning={} migration_save_pending={}",
            path,
            warning.is_some(),
            migrated
        );
        let deadline = migrated.then(|| Instant::now() + SAVE_DEBOUNCE);
        let pending = migrated.then(|| state.clone());
        (
            Self {
                path: Some(path),
                saved: state.clone(),
                observed: state.clone(),
                pending,
                pending_reason: SaveReason::Autosave,
                deadline,
                debounce: SAVE_DEBOUNCE,
                load_warning: warning,
                save_error: None,
            },
            state,
        )
    }

    pub(crate) fn unavailable(message: String) -> (Self, PersistedState) {
        log::warn!(
            target: "companion::persistence",
            "settings_unavailable reason={:?}",
            message
        );
        let state = PersistedState::default();
        (
            Self {
                path: None,
                saved: state.clone(),
                observed: state.clone(),
                pending: None,
                pending_reason: SaveReason::Autosave,
                deadline: None,
                debounce: SAVE_DEBOUNCE,
                load_warning: Some(format!(
                    "Saved state is unavailable: {message} Settings will remain in memory for this session."
                )),
                save_error: None,
            },
            state,
        )
    }

    pub(crate) fn load_warning(&self) -> Option<&str> {
        self.load_warning.as_deref()
    }

    pub(crate) fn save_error(&self) -> Option<&str> {
        self.save_error.as_deref()
    }
    pub(crate) fn open_config_directory(&self) -> Result<(), String> {
        self.open_config_directory_with(open_directory)
    }

    fn open_config_directory_with(
        &self,
        opener: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        let state_path = self
            .path
            .as_deref()
            .ok_or_else(|| "No per-user config folder is available.".to_owned())?;
        let directory = state_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| {
                format!(
                    "The saved-state path {} has no containing folder.",
                    state_path.display()
                )
            })?;
        fs::create_dir_all(directory).map_err(|error| {
            format!(
                "Could not create the config folder {}: {error}",
                directory.display()
            )
        })?;
        set_private_directory_permissions(directory)?;
        opener(directory)
    }

    pub(crate) fn observe(&mut self, state: PersistedState, now: Instant) -> Option<Duration> {
        if state != self.observed {
            self.observed = state.clone();
            if state == self.saved {
                self.pending = None;
                self.deadline = None;
                self.save_error = None;
                log::debug!(
                    target: "companion::persistence",
                    "settings_autosave_cancelled reason=reverted_to_saved"
                );
            } else {
                let coalesced = self.pending.is_some();
                self.pending = Some(state);
                self.pending_reason = SaveReason::Autosave;
                self.deadline = Some(now + self.debounce);
                log::debug!(
                    target: "companion::persistence",
                    "settings_autosave_scheduled coalesced={coalesced}"
                );
            }
        } else if self.pending.is_some() && self.deadline.is_none() {
            self.deadline = Some(now + self.debounce);
            log::debug!(
                target: "companion::persistence",
                "settings_autosave_rescheduled"
            );
        }

        if self.deadline.is_some_and(|deadline| deadline <= now) {
            let state = self
                .pending
                .clone()
                .expect("a save deadline requires pending state");
            let reason = self.pending_reason;
            if self.commit(state, reason).is_err() {
                self.deadline = Some(now + self.debounce);
            }
        }

        self.deadline
            .map(|deadline| deadline.saturating_duration_since(now))
    }

    pub(crate) fn save_reset_now(&mut self, state: PersistedState) -> Result<(), ()> {
        log::info!(target: "companion::persistence", "settings_reset_save_started");
        self.observed = state.clone();
        self.commit(state, SaveReason::Reset)
    }

    pub(crate) fn flush(&mut self, state: PersistedState) -> Result<(), ()> {
        if state == self.saved && self.pending.is_none() {
            log::debug!(
                target: "companion::persistence",
                "settings_flush_noop reason=clean"
            );
            return Ok(());
        }
        log::debug!(target: "companion::persistence", "settings_flush_started");
        self.observed = state.clone();
        let reason = self
            .pending
            .as_ref()
            .map(|_| self.pending_reason)
            .unwrap_or(SaveReason::Autosave);
        self.commit(state, reason)
    }

    fn commit(&mut self, state: PersistedState, reason: SaveReason) -> Result<(), ()> {
        let result = self
            .path
            .as_deref()
            .ok_or_else(|| "No per-user saved-state path is available.".to_owned())
            .and_then(|path| write_state(path, &state));
        match result {
            Ok(()) => {
                let recovered = self.save_error.is_some();
                self.saved = state;
                self.pending = None;
                self.deadline = None;
                self.save_error = None;
                log::info!(
                    target: "companion::persistence",
                    "settings_save_committed reason={} recovered={}",
                    match reason {
                        SaveReason::Autosave => "autosave",
                        SaveReason::Reset => "reset",
                    },
                    recovered
                );
                Ok(())
            }
            Err(error) => {
                let first_failure = self.save_error.is_none();
                self.pending = Some(state);
                self.pending_reason = reason;
                self.deadline = None;
                self.save_error = Some(match reason {
                    SaveReason::Autosave => format!(
                        "Could not save settings: {error} Changes remain unsaved and will be retried."
                    ),
                    SaveReason::Reset => format!(
                        "Current settings were reset in memory, but saved state could not be replaced: {error} The previous disk state may return after restart."
                    ),
                });
                if first_failure {
                    log::warn!(
                        target: "companion::persistence",
                        "settings_save_failed reason={} error={:?}",
                        match reason {
                            SaveReason::Autosave => "autosave",
                            SaveReason::Reset => "reset",
                        },
                        error
                    );
                }
                Err(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ShockMode;
    use crate::app::AbilityFilter;
    use std::collections::BTreeSet;

    #[test]
    fn config_folder_action_creates_and_opens_the_state_directory() {
        let root = tempfile::tempdir().unwrap();
        let state_path = root.path().join("nested").join("state.json");
        let expected_directory = state_path.parent().unwrap().to_owned();
        let (persistence, _) = Persistence::open(state_path);
        let opened = std::cell::Cell::new(false);
        persistence
            .open_config_directory_with(|directory| {
                assert_eq!(directory, expected_directory);
                assert!(directory.is_dir());
                opened.set(true);
                Ok(())
            })
            .unwrap();
        assert!(opened.get());
    }

    #[test]
    fn schema_four_round_trip_preserves_provider_and_action_banks() {
        let mut app = AppState::default();
        app.provider = ProviderKind::OpenShock;
        app.provider_settings.pishock.username = "pi-user".into();
        app.provider_settings.pishock.api_key = "pi-key".into();
        app.provider_settings.openshock.token = "open-token".into();
        app.preferred_target = Some(TargetId::OpenShock("group-id".into()));
        app.triggers.death.actions.shock.mode = ShockMode::Fixed;
        app.triggers.death.actions.shock.fixed.intensity = 43.0;
        app.triggers.ability_use.trigger.enabled = true;
        app.triggers.ability_use.ability_filter = AbilityFilter::Selected(BTreeSet::from([1, 3]));
        let persisted = PersistedState::from_app(&app);
        assert_eq!(persisted.schema_version, SCHEMA_VERSION);
        let restored =
            serde_json::from_str::<PersistedState>(&serde_json::to_string(&persisted).unwrap())
                .unwrap()
                .restore_app();
        assert_eq!(restored.provider, ProviderKind::OpenShock);
        assert_eq!(restored.provider_settings, app.provider_settings);
        assert_eq!(restored.preferred_target, app.preferred_target);
        assert_eq!(
            restored.triggers.death.actions.shock,
            app.triggers.death.actions.shock
        );
        assert_eq!(
            restored.triggers.ability_use.ability_filter,
            app.triggers.ability_use.ability_filter
        );
    }

    #[test]
    fn schema_three_fixture_migrates_all_provider_and_action_profiles() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        std::fs::write(
            &path,
            include_str!("../fixtures/provider-foundation-schema3.json"),
        )
        .unwrap();
        let outcome = load_from_path(&path);
        assert!(outcome.migrated);
        let restored = outcome.state.restore_app();
        assert_eq!(restored.provider, ProviderKind::OpenShock);
        assert_eq!(restored.provider_settings.pishock.api_key, "fixture-pi-key");
        assert_eq!(
            restored.provider_settings.openshock.token,
            "fixture-open-token"
        );
        assert_eq!(
            restored.preferred_target,
            Some(TargetId::OpenShock("fixture-group".into()))
        );
        assert_eq!(restored.triggers.death.actions.shock.fixed.intensity, 33.0);
        assert_eq!(
            restored.triggers.ability_use.ability_filter,
            AbilityFilter::Selected(BTreeSet::from([1, 3]))
        );
        assert_eq!(
            restored
                .triggers
                .ability_cooldown_ready
                .trigger
                .actions
                .shock
                .fixed
                .intensity,
            66.0
        );
        assert_eq!(restored.log_path, "/fixture/console.log");
        let (mut persistence, migrated) = Persistence::open(path.clone());
        assert_eq!(persistence.pending, Some(migrated.clone()));
        persistence.flush(migrated).unwrap();
        let rewritten: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(rewritten["schema_version"], SCHEMA_VERSION);
        assert_eq!(
            rewritten["provider_settings"]["pishock"]["api_key"],
            "fixture-pi-key"
        );
        assert_eq!(
            rewritten["triggers"]["local_player_death"]["actions"]["shock"]["fixed"]["intensity"],
            33.0
        );
    }

    #[test]
    fn normalization_is_independent_for_each_action_profile() {
        let mut state = PersistedState::default();
        state
            .triggers
            .local_player_death
            .actions
            .shock
            .interval
            .minimum_intensity = 120.0;
        state
            .triggers
            .ability_used
            .trigger
            .actions
            .shock
            .fixed
            .duration_seconds = 9.0;
        state
            .triggers
            .ability_cooldown_ready
            .trigger
            .actions
            .shock
            .interval
            .maximum_duration_seconds = 0.1;
        let normalized = state.normalized().unwrap();
        assert_eq!(
            normalized
                .triggers
                .local_player_death
                .actions
                .shock
                .interval
                .minimum_intensity,
            MAX_SHOCK_INTENSITY
        );
        assert_eq!(
            normalized
                .triggers
                .ability_used
                .trigger
                .actions
                .shock
                .fixed
                .duration_seconds,
            MAX_SHOCK_DURATION
        );
        assert_eq!(
            normalized
                .triggers
                .ability_cooldown_ready
                .trigger
                .actions
                .shock
                .interval
                .maximum_duration_seconds,
            MIN_SHOCK_DURATION
        );
    }
    #[test]
    fn schema_four_round_trip_preserves_all_setup_profiles_targets_filters_and_actions() {
        let mut original = AppState::default();
        original.provider = ProviderKind::OpenShock;
        original.provider_settings.pishock.username = "pi-user".to_owned();
        original.provider_settings.pishock.api_key = "pi-key".to_owned();
        original.provider_settings.openshock.token = "open-token".to_owned();
        original.preferred_target = Some(TargetId::OpenShock("group-id".to_owned()));
        original.triggers.death.enabled = false;
        original.triggers.death.actions.shock.mode = ShockMode::Fixed;
        original.triggers.death.actions.shock.fixed.intensity = 43.0;
        original.triggers.death.actions.shock.fixed.duration_seconds = 1.4;
        original.triggers.ability_use.trigger.enabled = true;
        original
            .triggers
            .ability_use
            .trigger
            .actions
            .shock
            .interval
            .minimum_intensity = 11.0;
        original
            .triggers
            .ability_use
            .trigger
            .actions
            .shock
            .interval
            .maximum_intensity = 72.0;
        original.triggers.ability_use.ability_filter =
            AbilityFilter::Selected(BTreeSet::from([1, 4]));
        original.triggers.ability_cooldown_ready.trigger.enabled = true;
        original
            .triggers
            .ability_cooldown_ready
            .trigger
            .actions
            .shock
            .fixed
            .intensity = 87.0;
        original.triggers.ability_cooldown_ready.ability_filter = AbilityFilter::All;
        original.log_path = "/logs/console.log".to_owned();
        let persisted = PersistedState::from_app(&original);
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string_pretty(&persisted).unwrap()).unwrap();
        assert_eq!(value["schema_version"], SCHEMA_VERSION);
        assert_eq!(value["provider"], "openshock");
        assert_eq!(value["preferred_target"]["id"], "group-id");
        assert_eq!(
            value["triggers"]["local_player_death"]["actions"]["shock"]["fixed"]["intensity"],
            43.0
        );
        let restored = serde_json::from_value::<PersistedState>(value)
            .unwrap()
            .normalized()
            .unwrap()
            .restore_app();
        assert_eq!(restored.provider_settings, original.provider_settings);
        assert_eq!(restored.preferred_target, original.preferred_target);
        assert_eq!(restored.triggers, original.triggers);
        assert_eq!(restored.log_path, original.log_path);
    }

    #[test]
    fn schema_six_round_trip_preserves_kill_and_assist_settings() {
        let mut app = AppState::default();
        app.triggers.kill.enabled = true;
        app.triggers.kill.actions.shock.mode = ShockMode::Fixed;
        app.triggers.kill.actions.shock.fixed.intensity = 27.0;
        app.triggers.assist.enabled = true;
        app.triggers.assist.actions.shock.fixed.duration_seconds = 0.8;
        let persisted = PersistedState::from_app(&app);
        assert_eq!(persisted.schema_version, SCHEMA_VERSION);
        let restored =
            serde_json::from_str::<PersistedState>(&serde_json::to_string(&persisted).unwrap())
                .unwrap()
                .restore_app();
        assert!(restored.triggers.kill.enabled);
        assert_eq!(
            restored.triggers.kill.actions.shock,
            app.triggers.kill.actions.shock
        );
        assert!(restored.triggers.assist.enabled);
        assert_eq!(
            restored.triggers.assist.actions.shock,
            app.triggers.assist.actions.shock
        );
    }

    #[test]
    fn schema_five_migration_defaults_kill_and_assist_to_disabled() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let shock = serde_json::json!({
            "mode": "interval",
            "interval": {
                "minimum_intensity": 10.0,
                "maximum_intensity": 20.0,
                "minimum_duration_seconds": 0.2,
                "maximum_duration_seconds": 0.5
            },
            "fixed": { "intensity": 15.0, "duration_seconds": 0.3 }
        });
        let schema_five = serde_json::json!({
            "schema_version": 5,
            "provider": "pishock",
            "provider_settings": {
                "pishock": { "username": "pi-user", "api_key": "pi-key" },
                "openshock": { "token": "" },
                "lovense": { "domain": "192.168.1.2", "http_port": 30010 }
            },
            "preferred_target": null,
            "triggers": {
                "local_player_death": {
                    "enabled": true,
                    "actions": { "shock": shock.clone() }
                },
                "ability_used": {
                    "trigger": {
                        "enabled": false,
                        "actions": { "shock": shock.clone() }
                    },
                    "ability_filter": { "mode": "all", "slots": [] }
                },
                "ability_cooldown_ready": {
                    "trigger": {
                        "enabled": false,
                        "actions": { "shock": shock.clone() }
                    },
                    "ability_filter": { "mode": "all", "slots": [] }
                }
            },
            "log_path": ""
        });
        fs::write(&path, serde_json::to_vec_pretty(&schema_five).unwrap()).unwrap();
        let outcome = load_from_path(&path);
        assert!(outcome.migrated);
        let restored = outcome.state.restore_app();
        assert!(restored.triggers.death.enabled);
        assert!(!restored.triggers.kill.enabled);
        assert!(!restored.triggers.assist.enabled);
        assert_eq!(
            restored.triggers.kill.actions.shock,
            restored.triggers.death.actions.shock
        );
    }

    #[test]
    fn schema_one_migration_clones_shock_and_preserves_legacy_behavior() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let schema_one = serde_json::json!({
            "schema_version": 1,
            "provider": "openshock",
            "credentials": {
                "pishock": { "username": "pi-user", "api_key": "pi-key" },
                "openshock": { "token": "open-token" }
            },
            "preferred_target": { "provider": "openshock", "id": "group-id" },
            "shock": {
                "mode": "fixed",
                "interval": {
                    "minimum_intensity": 11.0,
                    "maximum_intensity": 72.0,
                    "minimum_duration_seconds": 0.5,
                    "maximum_duration_seconds": 2.6
                },
                "fixed": { "intensity": 43.0, "duration_seconds": 1.4 }
            },
            "log_path": "/logs/console.log"
        });
        fs::write(&path, serde_json::to_vec_pretty(&schema_one).unwrap()).unwrap();
        let outcome = load_from_path(&path);
        assert!(outcome.warning.is_none());
        let restored = outcome.state.restore_app();
        assert_eq!(restored.provider, ProviderKind::OpenShock);
        assert_eq!(restored.provider_settings.pishock.username, "pi-user");
        assert_eq!(restored.provider_settings.openshock.token, "open-token");
        assert_eq!(
            restored.preferred_target,
            Some(TargetId::OpenShock("group-id".to_owned()))
        );
        assert_eq!(restored.triggers.death.actions.shock.mode, ShockMode::Fixed);
        assert_eq!(restored.triggers.death.actions.shock.fixed.intensity, 43.0);
        assert_eq!(
            restored.triggers.ability_use.trigger.actions.shock,
            restored.triggers.death.actions.shock
        );
        assert_eq!(
            restored
                .triggers
                .ability_cooldown_ready
                .trigger
                .actions
                .shock,
            restored.triggers.death.actions.shock
        );
        assert_eq!(restored.log_path, "/logs/console.log");
        let (mut persistence, migrated) = Persistence::open(path.clone());
        assert_eq!(persistence.pending, Some(migrated.clone()));
        persistence.flush(migrated).unwrap();
        let rewritten: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(rewritten["schema_version"], SCHEMA_VERSION);
        assert!(rewritten.get("shock").is_none());
        assert!(rewritten["triggers"]["local_player_death"]["actions"]["shock"].is_object());
    }

    #[test]
    fn schema_two_migration_preserves_toggles_and_clones_shared_shock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let schema_two = serde_json::json!({
            "schema_version": 2,
            "provider": "pishock",
            "credentials": {
                "pishock": { "username": "pi-user", "api_key": "pi-key" },
                "openshock": { "token": "open-token" }
            },
            "preferred_target": null,
            "shock": {
                "mode": "fixed",
                "interval": {
                    "minimum_intensity": 12.0,
                    "maximum_intensity": 34.0,
                    "minimum_duration_seconds": 0.7,
                    "maximum_duration_seconds": 2.1
                },
                "fixed": { "intensity": 56.0, "duration_seconds": 1.8 }
            },
            "triggers": {
                "local_player_death": false,
                "ability_used": true,
                "ability_cooldown_ready": true
            },
            "log_path": "/old/log"
        });
        fs::write(&path, serde_json::to_vec_pretty(&schema_two).unwrap()).unwrap();
        let restored = load_from_path(&path).state.restore_app();
        assert!(!restored.triggers.death.enabled);
        assert!(restored.triggers.ability_use.trigger.enabled);
        assert!(restored.triggers.ability_cooldown_ready.trigger.enabled);
        assert_eq!(
            restored.triggers.death.actions.shock,
            restored.triggers.ability_use.trigger.actions.shock
        );
        assert_eq!(
            restored.triggers.death.actions.shock,
            restored
                .triggers
                .ability_cooldown_ready
                .trigger
                .actions
                .shock
        );
        assert_eq!(restored.triggers.death.actions.shock.fixed.intensity, 56.0);
    }

    #[test]
    fn missing_file_loads_current_defaults_without_warning() {
        let directory = tempfile::tempdir().unwrap();
        let outcome = load_from_path(&directory.path().join("state.json"));
        assert_eq!(outcome.state, PersistedState::default());
        assert!(outcome.warning.is_none());
    }

    #[test]
    fn strict_unknown_fields_are_preserved_as_invalid_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let mut value = serde_json::to_value(PersistedState::default()).unwrap();
        value["triggers"]["ability_used"]["unexpected"] = serde_json::json!(true);
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        let outcome = load_from_path(&path);
        assert_eq!(outcome.state, PersistedState::default());
        assert!(outcome.warning.unwrap().contains("unknown field"));
        assert!(!path.exists());
    }

    #[test]
    fn corrupt_file_is_preserved_before_defaults_are_returned() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        fs::write(&path, "{not json").unwrap();
        let outcome = load_from_path(&path);
        assert_eq!(outcome.state, PersistedState::default());
        assert!(outcome.warning.as_deref().unwrap().contains("preserved"));
        assert!(!path.exists());
        let backups = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        assert!(
            backups[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("state.invalid-")
        );
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), "{not json");
    }

    #[test]
    fn normalization_canonicalizes_each_profile_and_filter_independently() {
        let mut state = PersistedState::default();
        state
            .triggers
            .local_player_death
            .actions
            .shock
            .interval
            .minimum_intensity = 120.0;
        state
            .triggers
            .local_player_death
            .actions
            .shock
            .interval
            .maximum_intensity = -5.0;
        state
            .triggers
            .ability_used
            .trigger
            .actions
            .shock
            .fixed
            .duration_seconds = 9.0;
        state.triggers.ability_used.ability_filter = PersistedAbilityFilter {
            mode: PersistedAbilityFilterMode::Selected,
            slots: vec![4, 0, 2, 4, 2],
        };
        state.triggers.ability_cooldown_ready.ability_filter = PersistedAbilityFilter {
            mode: PersistedAbilityFilterMode::All,
            slots: vec![1, 7],
        };
        let normalized = state.normalized().unwrap();
        assert_eq!(
            normalized
                .triggers
                .local_player_death
                .actions
                .shock
                .interval
                .minimum_intensity,
            MAX_SHOCK_INTENSITY
        );
        assert_eq!(
            normalized
                .triggers
                .local_player_death
                .actions
                .shock
                .interval
                .maximum_intensity,
            MAX_SHOCK_INTENSITY
        );
        assert_eq!(
            normalized
                .triggers
                .ability_used
                .trigger
                .actions
                .shock
                .fixed
                .duration_seconds,
            MAX_SHOCK_DURATION
        );
        assert_eq!(
            normalized.triggers.ability_used.ability_filter.slots,
            vec![2, 4]
        );
        assert!(
            normalized
                .triggers
                .ability_cooldown_ready
                .ability_filter
                .slots
                .is_empty()
        );
    }

    #[test]
    fn debounce_writes_once_and_flush_writes_immediately() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let (mut persistence, initial) = Persistence::open(path.clone());
        let mut changed = initial.clone();
        changed.log_path = "/changed".to_owned();
        let start = Instant::now();
        assert_eq!(
            persistence.observe(changed.clone(), start),
            Some(SAVE_DEBOUNCE)
        );
        assert!(!path.exists());
        assert_eq!(
            persistence.observe(changed.clone(), start + SAVE_DEBOUNCE),
            None
        );
        assert_eq!(
            serde_json::from_str::<PersistedState>(&fs::read_to_string(&path).unwrap()).unwrap(),
            changed
        );
        changed.log_path = "/exit-flush".to_owned();
        persistence.observe(changed.clone(), start + SAVE_DEBOUNCE * 2);
        persistence.flush(changed.clone()).unwrap();
        assert_eq!(
            serde_json::from_str::<PersistedState>(&fs::read_to_string(&path).unwrap()).unwrap(),
            changed
        );
        changed.log_path = "/save-now".to_owned();
        persistence.save_reset_now(changed.clone()).unwrap();
        assert_eq!(
            serde_json::from_str::<PersistedState>(&fs::read_to_string(&path).unwrap()).unwrap(),
            changed
        );
    }

    #[test]
    fn unsupported_schema_is_backed_up_like_malformed_json() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let state = PersistedState {
            schema_version: SCHEMA_VERSION + 1,
            ..PersistedState::default()
        };
        fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
        let outcome = load_from_path(&path);
        assert_eq!(outcome.state, PersistedState::default());
        assert!(
            outcome
                .warning
                .unwrap()
                .contains("unsupported schema version")
        );
        assert!(!path.exists());
    }

    #[test]
    fn failed_reset_save_stays_dirty_and_reports_previous_disk_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        fs::create_dir(&path).unwrap();
        let (mut persistence, mut state) = Persistence::open(path);
        state.log_path = "/reset-in-memory".to_owned();
        assert!(persistence.save_reset_now(state.clone()).is_err());
        assert_eq!(persistence.pending, Some(state));
        assert!(
            persistence
                .save_error()
                .unwrap()
                .contains("previous disk state may return")
        );
    }
}
