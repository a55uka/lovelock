use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

use crate::action::{ResolvedVibrateAction, VibrateActionSettings};
use crate::bridge_listener::{
    AbilityTrigger, BridgeEvent, ConsoleLogListener, CountTrigger, ListenerPhase, ListenerStatus,
    LocalPlayerDeath, ModVersionObservation,
};
use crate::deadlock_path::{self, Detection, DetectionError};
use crate::logging::{LogSnapshot, LogStore};
use crate::persistence::{PersistedState, Persistence, default_state_path};
use crate::provider::{ProviderError, ProviderSettings, ProviderTarget, TargetId};
use crate::providers::{ConnectedProvider, ProviderKind};
use crate::version_check::{
    COMPANION_RELEASE_URL, LATEST_RELEASE_URL, MOD_RELEASE_URL, VersionCheckOwner,
    VersionCheckState, WarningSelection, app_version, select_warnings,
};
use egui::{Color32, TextEdit, Ui};

pub(crate) const ACTION_QUEUE_CAPACITY: usize = 10;
pub(crate) const MAX_ACTION_QUEUE_AGE: Duration = Duration::from_secs(30);
const ACTION_KIND_LABEL: &str = "vibrate";

#[derive(Clone, Debug, PartialEq)]
pub struct TriggerSettings {
    pub enabled: bool,
    pub actions: VibrateActionSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AbilityTriggerSettings {
    pub trigger: TriggerSettings,
    pub ability_filter: AbilityFilter,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AbilityFilter {
    All,
    Selected(BTreeSet<u32>),
}

impl Default for AbilityFilter {
    fn default() -> Self {
        Self::All
    }
}

impl AbilityFilter {
    pub fn accepts(&self, ability_slot: u32) -> bool {
        match self {
            Self::All => true,
            Self::Selected(slots) => slots.contains(&ability_slot),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TriggerSettingsSet {
    pub death: TriggerSettings,
    pub kill: TriggerSettings,
    pub assist: TriggerSettings,
    pub ability_use: AbilityTriggerSettings,
    pub ability_cooldown_ready: AbilityTriggerSettings,
}
impl Default for TriggerSettingsSet {
    fn default() -> Self {
        let actions = VibrateActionSettings::default();
        Self {
            death: TriggerSettings {
                enabled: true,
                actions: actions.clone(),
            },
            kill: TriggerSettings {
                enabled: false,
                actions: actions.clone(),
            },
            assist: TriggerSettings {
                enabled: false,
                actions: actions.clone(),
            },
            ability_use: AbilityTriggerSettings {
                trigger: TriggerSettings {
                    enabled: false,
                    actions: actions.clone(),
                },
                ability_filter: AbilityFilter::All,
            },
            ability_cooldown_ready: AbilityTriggerSettings {
                trigger: TriggerSettings {
                    enabled: false,
                    actions,
                },
                ability_filter: AbilityFilter::All,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CredentialState {
    #[default]
    Unknown,
    Testing,
    Valid,
    Invalid,
}
impl CredentialState {
    fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Not tested yet",
            Self::Testing => "Testing connection…",
            Self::Valid => "Connected",
            Self::Invalid => "Connection failed",
        }
    }
    fn tone(self) -> crate::theme::BadgeTone {
        match self {
            Self::Unknown | Self::Testing => crate::theme::BadgeTone::Neutral,
            Self::Valid => crate::theme::BadgeTone::Success,
            Self::Invalid => crate::theme::BadgeTone::Danger,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LogDetectionStatus {
    Found,
    NotCreated,
    Failed(String),
}
impl LogDetectionStatus {
    fn label(&self) -> &str {
        match self {
            Self::Found => "Found Deadlock console.log.",
            Self::NotCreated => {
                "Deadlock is installed, but console.log has not been created. Add -condebug to Deadlock's Steam launch options, then launch the game."
            }
            Self::Failed(message) => message,
        }
    }
    fn tone(&self) -> crate::theme::BadgeTone {
        match self {
            Self::Found => crate::theme::BadgeTone::Success,
            Self::NotCreated => crate::theme::BadgeTone::Warning,
            Self::Failed(_) => crate::theme::BadgeTone::Danger,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogListenerStartContext {
    Manual,
    Startup,
}

impl LogListenerStartContext {
    fn label(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Startup => "startup",
        }
    }
}

type ConnectionResult = Result<(ConnectedProvider, Vec<ProviderTarget>), ProviderError>;
type TestActionResult = Result<(), ProviderError>;
fn provider_error_kind(error: &ProviderError) -> &'static str {
    match error {
        ProviderError::Lovense(_) => "lovense",
        ProviderError::Buttplug(_) => "buttplug",
        ProviderError::InvalidSetup => "invalid_setup",
        ProviderError::NotConnected => "not_connected",
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum TestActionStatus {
    Sending,
    Sent,
    Failed(String),
}
impl TestActionStatus {
    fn label(&self) -> &str {
        match self {
            Self::Sending => "Sending test vibration…",
            Self::Sent => "Test vibration sent.",
            Self::Failed(message) => message,
        }
    }
    fn tone(&self) -> crate::theme::BadgeTone {
        match self {
            Self::Sending => crate::theme::BadgeTone::Neutral,
            Self::Sent => crate::theme::BadgeTone::Success,
            Self::Failed(_) => crate::theme::BadgeTone::Danger,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TriggerKind {
    Death,
    Kill,
    Assist,
    AbilityUse,
    AbilityCooldownReady,
}

impl TriggerKind {
    fn label(self) -> &'static str {
        match self {
            Self::Death => "death",
            Self::Kill => "kill",
            Self::Assist => "assist",
            Self::AbilityUse => "ability use",
            Self::AbilityCooldownReady => "ability cooldown ready",
        }
    }
}

impl TriggerSettingsSet {
    fn get(&self, kind: TriggerKind) -> &TriggerSettings {
        match kind {
            TriggerKind::Death => &self.death,
            TriggerKind::Kill => &self.kill,
            TriggerKind::Assist => &self.assist,
            TriggerKind::AbilityUse => &self.ability_use.trigger,
            TriggerKind::AbilityCooldownReady => &self.ability_cooldown_ready.trigger,
        }
    }

    fn get_mut(&mut self, kind: TriggerKind) -> &mut TriggerSettings {
        match kind {
            TriggerKind::Death => &mut self.death,
            TriggerKind::Kill => &mut self.kill,
            TriggerKind::Assist => &mut self.assist,
            TriggerKind::AbilityUse => &mut self.ability_use.trigger,
            TriggerKind::AbilityCooldownReady => &mut self.ability_cooldown_ready.trigger,
        }
    }

    fn ability_filter(&self, kind: TriggerKind) -> Option<&AbilityFilter> {
        match kind {
            TriggerKind::Death | TriggerKind::Kill | TriggerKind::Assist => None,
            TriggerKind::AbilityUse => Some(&self.ability_use.ability_filter),
            TriggerKind::AbilityCooldownReady => Some(&self.ability_cooldown_ready.ability_filter),
        }
    }

    fn ability_filter_mut(&mut self, kind: TriggerKind) -> Option<&mut AbilityFilter> {
        match kind {
            TriggerKind::Death | TriggerKind::Kill | TriggerKind::Assist => None,
            TriggerKind::AbilityUse => Some(&mut self.ability_use.ability_filter),
            TriggerKind::AbilityCooldownReady => {
                Some(&mut self.ability_cooldown_ready.ability_filter)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum AppSection {
    #[default]
    Setup,
    Effects,
    GameConnection,
    Donate,
}

impl AppSection {
    fn label(self) -> &'static str {
        match self {
            Self::Setup => "Setup",
            Self::Effects => "Effects",
            Self::GameConnection => "Game connection",
            Self::Donate => "Donate",
        }
    }
}

const KOFI_URL: &str = "https://ko-fi.com/asteriaxo";
const VOLC_PROFILE_URL: &str = "https://gamebanana.com/members/5741016";

#[derive(Clone, Debug, Eq, PartialEq)]
struct TriggerIdentity {
    kind: TriggerKind,
    session_id: String,
    sequence: u64,
    client_time_ms: u64,
    detection: String,
    ability_slot: Option<u32>,
    ability_name: Option<String>,
    charges_before: Option<u64>,
    charges_after: Option<u64>,
}

impl TriggerIdentity {
    fn from_death(death: LocalPlayerDeath) -> Self {
        Self {
            kind: TriggerKind::Death,
            session_id: death.session_id,
            sequence: death.sequence,
            client_time_ms: death.client_time_ms,
            detection: death.detection,
            ability_slot: None,
            ability_name: None,
            charges_before: None,
            charges_after: None,
        }
    }

    fn from_ability(kind: TriggerKind, ability: AbilityTrigger) -> Self {
        Self {
            kind,
            session_id: ability.session_id,
            sequence: ability.sequence,
            client_time_ms: ability.client_time_ms,
            detection: ability.detection,
            ability_slot: Some(ability.ability_slot),
            ability_name: ability.ability_name,
            charges_before: ability.charges_before,
            charges_after: ability.charges_after,
        }
    }

    fn from_count(kind: TriggerKind, count: CountTrigger) -> Self {
        Self {
            kind,
            session_id: count.session_id,
            sequence: count.sequence,
            client_time_ms: count.client_time_ms,
            detection: count.detection,
            ability_slot: None,
            ability_name: None,
            charges_before: count.count_before,
            charges_after: count.count_after,
        }
    }

    fn status_description(&self) -> String {
        if matches!(
            self.kind,
            TriggerKind::Death | TriggerKind::Kill | TriggerKind::Assist
        ) {
            return format!(
                "{} {}#{}",
                self.kind.label(),
                self.session_id,
                self.sequence
            );
        }
        let name = self
            .ability_name
            .as_deref()
            .map(|name| format!(" ({name})"))
            .unwrap_or_default();
        let charges = match (self.charges_before, self.charges_after) {
            (Some(before), Some(after)) => format!(", charges {before}→{after}"),
            _ => String::new(),
        };
        format!(
            "{} slot {}{name}, detection {}{charges}, {}#{}",
            self.kind.label(),
            self.ability_slot.unwrap_or_default(),
            self.detection,
            self.session_id,
            self.sequence
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActionRequest {
    target: Option<ProviderTarget>,
    resolved: ResolvedVibrateAction,
    trigger: TriggerIdentity,
    queued_at: Instant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActionSnapshot {
    target: Option<ProviderTarget>,
    resolved: Option<ResolvedVibrateAction>,
    trigger: TriggerIdentity,
}
impl ActionSnapshot {
    fn from_request(request: &ActionRequest) -> Self {
        Self {
            target: request.target.clone(),
            resolved: Some(request.resolved),
            trigger: request.trigger.clone(),
        }
    }
}

struct ActionJob {
    client: Arc<ConnectedProvider>,
    request: ActionRequest,
}

#[derive(Debug)]
enum ActionCompletionResult {
    Completed(Result<(), ProviderError>),
    Skipped { reason: &'static str },
}

struct ActionCompletion {
    request: ActionRequest,
    result: ActionCompletionResult,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActionEnqueueResult {
    Accepted,
    Full,
    Disconnected,
}

fn action_job_expired_at(queued_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(queued_at) >= MAX_ACTION_QUEUE_AGE
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ActionStatus {
    Sending(ActionRequest),
    Sent(ActionRequest),
    Failed {
        request: ActionRequest,
        error: String,
    },
    Skipped {
        snapshot: ActionSnapshot,
        reason: String,
    },
}
impl ActionStatus {
    fn snapshot(&self) -> ActionSnapshot {
        match self {
            Self::Sending(request) | Self::Sent(request) => ActionSnapshot::from_request(request),
            Self::Failed { request, .. } => ActionSnapshot::from_request(request),
            Self::Skipped { snapshot, .. } => snapshot.clone(),
        }
    }
    fn label(&self) -> String {
        let snapshot = self.snapshot();
        let target = snapshot
            .target
            .as_ref()
            .map(|target| target.name())
            .unwrap_or("no target");
        let action = ACTION_KIND_LABEL;
        let resolved = snapshot
            .resolved
            .map(ResolvedVibrateAction::summary)
            .unwrap_or_else(|| "settings unavailable".to_owned());
        let trigger = snapshot.trigger.status_description();
        match self {
            Self::Sending(_) => {
                format!("Sending {action} to {target} at {resolved} ({trigger})…")
            }
            Self::Sent(_) => {
                format!("{action} sent to {target} at {resolved} ({trigger}).")
            }
            Self::Failed { error, .. } => {
                format!("{action} failed for {target} at {resolved} ({trigger}): {error}")
            }
            Self::Skipped { reason, .. } => {
                format!("{action} skipped for {target} at {resolved} ({trigger}): {reason}")
            }
        }
    }
    fn color(&self) -> [f32; 4] {
        match self {
            Self::Sending(_) => [0.65, 0.65, 0.65, 1.0],
            Self::Sent(_) => [0.30, 0.78, 0.42, 1.0],
            Self::Failed { .. } => [0.92, 0.32, 0.28, 1.0],
            Self::Skipped { .. } => [0.92, 0.68, 0.22, 1.0],
        }
    }
}

fn spawn_action_worker() -> (SyncSender<ActionJob>, Receiver<ActionCompletion>) {
    let (job_sender, job_receiver) = mpsc::sync_channel::<ActionJob>(ACTION_QUEUE_CAPACITY);
    let (completion_sender, completion_receiver) = mpsc::channel::<ActionCompletion>();
    thread::spawn(move || {
        while let Ok(job) = job_receiver.recv() {
            let result = if action_job_expired_at(job.request.queued_at, Instant::now()) {
                ActionCompletionResult::Skipped { reason: "expired" }
            } else {
                ActionCompletionResult::Completed(
                    job.client
                        .execute(job.request.target.as_ref(), job.request.resolved),
                )
            };
            let _ = completion_sender.send(ActionCompletion {
                request: job.request,
                result,
            });
        }
    });
    (job_sender, completion_receiver)
}

pub struct AppState {
    pub provider_settings: ProviderSettings,
    pub selected_provider: ProviderKind,
    pub credential_state: CredentialState,
    pub devices: Vec<ProviderTarget>,
    pub selected_device: Option<TargetId>,
    pub preferred_target: Option<TargetId>,
    pub triggers: TriggerSettingsSet,
    pub log_path: String,
    client: Option<Arc<ConnectedProvider>>,
    connection_error: Option<String>,
    connection_result: Option<Receiver<ConnectionResult>>,
    device_refresh_result: Option<Receiver<Result<Vec<ProviderTarget>, ProviderError>>>,
    scan_result: Option<Receiver<Result<(), ProviderError>>>,
    scan_refresh_result: Option<Receiver<Result<Vec<ProviderTarget>, ProviderError>>>,
    scanning: bool,
    test_action_result: Option<Receiver<TestActionResult>>,
    test_action_status: Option<TestActionStatus>,
    action_sender: SyncSender<ActionJob>,
    action_result: Receiver<ActionCompletion>,
    action_in_flight: usize,
    action_status: Option<ActionStatus>,
    log_detection_status: Option<LogDetectionStatus>,
    bridge_listener: ConsoleLogListener,
    bridge_events: Option<Receiver<BridgeEvent>>,
    last_bridge_event: Option<BridgeEvent>,
    last_sequence: Option<(String, u64)>,
    last_scan_refresh: Option<std::time::Instant>,
    ability_catalog: BTreeMap<u32, Option<String>>,
    listener_action_error: Option<String>,
    selected_section: AppSection,
    selected_effect: TriggerKind,
    copy_source: TriggerKind,
    copy_feedback: Option<String>,
}
impl Default for AppState {
    fn default() -> Self {
        let (action_sender, action_result) = spawn_action_worker();
        Self {
            provider_settings: ProviderSettings::default(),
            selected_provider: ProviderKind::Lovense,
            credential_state: CredentialState::default(),
            devices: Vec::new(),
            selected_device: None,
            preferred_target: None,
            triggers: TriggerSettingsSet::default(),
            log_path: String::new(),
            client: None,
            connection_error: None,
            connection_result: None,
            device_refresh_result: None,
            scan_result: None,
            scan_refresh_result: None,
            scanning: false,
            test_action_result: None,
            test_action_status: None,
            action_sender,
            action_result,
            action_in_flight: 0,
            action_status: None,
            log_detection_status: None,
            bridge_listener: ConsoleLogListener::default(),
            bridge_events: None,
            last_bridge_event: None,
            last_sequence: None,
            last_scan_refresh: None,
            ability_catalog: BTreeMap::new(),
            listener_action_error: None,
            selected_section: AppSection::default(),
            selected_effect: TriggerKind::Death,
            copy_source: TriggerKind::AbilityUse,
            copy_feedback: None,
        }
    }
}
impl AppState {
    pub(crate) fn effective_provider_settings(&self) -> ProviderSettings {
        self.provider_settings.clone()
    }
    pub fn credentials_present(&self) -> bool {
        self.provider_settings.present()
    }
    pub fn selected_device(&self) -> Option<&ProviderTarget> {
        let selected = self.selected_device.as_ref()?;
        self.devices.iter().find(|device| device.id() == selected)
    }
    fn connection_in_progress(&self) -> bool {
        self.connection_result.is_some()
    }
    fn device_refresh_in_progress(&self) -> bool {
        self.device_refresh_result.is_some()
    }
    fn test_action_in_progress(&self) -> bool {
        self.test_action_result.is_some()
    }
    fn action_in_progress(&self) -> bool {
        self.action_in_flight != 0
    }
    pub(crate) fn is_busy(&self) -> bool {
        self.connection_in_progress()
            || self.device_refresh_in_progress()
            || self.test_action_in_progress()
            || self.action_in_progress()
    }
    pub(crate) fn reset_saved_state(&mut self) -> bool {
        if self.is_busy() {
            log::warn!(target: "companion::app", "settings_reset_skipped reason=busy");
            return false;
        }

        self.bridge_listener.stop();
        self.reset_connection();
        self.provider_settings = ProviderSettings::default();
        self.selected_provider = self.provider_settings.kind();
        self.preferred_target = None;
        self.triggers = TriggerSettingsSet::default();
        self.log_path.clear();
        self.last_sequence = None;
        self.ability_catalog.clear();
        self.listener_action_error = None;
        self.log_detection_status = None;
        self.bridge_listener = ConsoleLogListener::default();
        self.bridge_events = None;
        self.last_bridge_event = None;
        let (action_sender, action_result) = spawn_action_worker();
        self.action_sender = action_sender;
        self.action_result = action_result;
        self.copy_feedback = None;
        self.action_status = None;
        self.action_in_flight = 0;
        log::info!(target: "companion::app", "settings_reset_applied provider={provider}", provider = self.provider_settings.kind().log_label());
        true
    }
    #[cfg(test)]
    pub(crate) fn listener_is_running(&self) -> bool {
        self.bridge_listener.status().phase != ListenerPhase::Stopped
    }
    #[cfg(test)]
    pub(crate) fn runtime_trigger_and_action_state_is_clear(&self) -> bool {
        self.bridge_events.is_none()
            && self.last_bridge_event.is_none()
            && self.last_sequence.is_none()
            && self.ability_catalog.is_empty()
            && self.action_status.is_none()
            && self.action_in_flight == 0
    }

    fn reset_connection(&mut self) {
        if let Some(client) = self.client.take() {
            match Arc::try_unwrap(client) {
                Ok(client) => match client.disconnect() {
                    Ok(()) => log::info!(
                        target: "companion::app",
                        "provider_disconnected provider={provider} outcome=success",
                        provider = self.provider_settings.kind().log_label()
                    ),
                    Err(error) => log::warn!(
                        target: "companion::app",
                    "provider_disconnected provider={provider} outcome=failed error_kind={}",
                    provider_error_kind(&error),
                    provider = self.provider_settings.kind().log_label()
                    ),
                },
                Err(_) => log::debug!(
                    target: "companion::app",
                    "provider_disconnect_skipped provider={provider} reason=shared_client",
                    provider = self.provider_settings.kind().log_label()
                ),
            }
        }
        self.credential_state = CredentialState::Unknown;
        self.connection_result = None;
        self.device_refresh_result = None;
        self.scan_result = None;
        self.scan_refresh_result = None;
        self.scanning = false;
        self.last_scan_refresh = None;
        self.devices.clear();
        self.selected_device = None;
        self.connection_error = None;
        self.test_action_result = None;
        self.test_action_status = None;
    }
    fn select_provider(&mut self, kind: ProviderKind) {
        if kind == self.provider_settings.kind() {
            self.selected_provider = kind;
            return;
        }
        self.selected_provider = kind;
        self.provider_settings = match (kind, &self.provider_settings) {
            (ProviderKind::Lovense, ProviderSettings::Lovense(_)) => self.provider_settings.clone(),
            (ProviderKind::Lovense, _) => ProviderSettings::Lovense(Default::default()),
            (ProviderKind::Local, _) => ProviderSettings::Local(Default::default()),
            (ProviderKind::Intiface, ProviderSettings::Intiface(_)) => {
                self.provider_settings.clone()
            }
            (ProviderKind::Intiface, _) => ProviderSettings::Intiface(Default::default()),
        };
        self.reset_connection();
    }
    fn start_scanning(&mut self, context: Option<egui::Context>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel::<Result<(), ProviderError>>();
        self.scan_result = Some(receiver);
        self.scanning = true;
        self.last_scan_refresh = None;
        thread::spawn(move || {
            let result = client.start_scanning();
            let _ = sender.send(result);
            if let Some(ctx) = context {
                ctx.request_repaint();
            }
        });
    }
    fn stop_scanning(&mut self, context: Option<egui::Context>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel::<Result<(), ProviderError>>();
        self.scan_result = Some(receiver);
        self.scanning = false;
        self.last_scan_refresh = None;
        thread::spawn(move || {
            let result = client.stop_scanning();
            let _ = sender.send(result);
            if let Some(ctx) = context {
                ctx.request_repaint();
            }
        });
    }
    fn poll_scan(&mut self) {
        let Some(receiver) = &self.scan_result else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.scan_result = None;
                match result {
                    Ok(()) => {
                        if let Some(client) = &self.client {
                            match client.list_targets() {
                                Ok(devices) => self.apply_devices(devices),
                                Err(error) => self.connection_error = Some(error.user_message()),
                            }
                        }
                    }
                    Err(error) => {
                        self.connection_error = Some(error.user_message());
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.scan_result = None,
        }
    }
    fn auto_refresh_scanning_devices(&mut self, ctx: egui::Context) {
        if !self.scanning || self.client.is_none() {
            self.last_scan_refresh = None;
            return;
        }
        let now = std::time::Instant::now();
        let should_refresh = match self.last_scan_refresh {
            Some(last) => now.duration_since(last) >= std::time::Duration::from_secs(2),
            None => true,
        };
        if !should_refresh {
            return;
        }
        self.last_scan_refresh = Some(now);
        let Some(client) = self.client.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.scan_refresh_result = Some(receiver);
        thread::spawn(move || {
            let _ = sender.send(client.list_targets());
            ctx.request_repaint();
        });
    }
    fn poll_scan_refresh(&mut self) {
        let Some(receiver) = &self.scan_refresh_result else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(devices)) => {
                self.scan_refresh_result = None;
                self.apply_devices(devices);
            }
            Ok(Err(error)) => {
                self.scan_refresh_result = None;
                self.connection_error = Some(error.user_message());
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.scan_refresh_result = None,
        }
    }
    fn start_connection_test(&mut self, context: egui::Context) {
        let config = self.provider_settings.clone();
        log::info!(
            target: "companion::app",
            "connection_test_started provider={provider}",
            provider = self.provider_settings.kind().log_label()
        );
        self.reset_connection();
        let (sender, receiver) = mpsc::channel();
        self.credential_state = CredentialState::Testing;
        self.connection_error = None;
        self.connection_result = Some(receiver);
        thread::spawn(move || {
            let result = crate::providers::connect(&config).and_then(|client| {
                let devices = client.list_targets()?;
                Ok((client, devices))
            });
            let _ = sender.send(result);
            context.request_repaint();
        });
    }
    fn poll_connection_test(&mut self) {
        let Some(receiver) = &self.connection_result else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.connection_result = None;
                self.apply_connection_result(result);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                log::error!(
                    target: "companion::app",
                    "connection_worker_failed provider={provider} reason=channel_closed",
                    provider = self.provider_settings.kind().log_label()
                );
                self.connection_result = None;
                self.apply_connection_error(ProviderError::NotConnected);
            }
        }
    }

    fn start_device_refresh(&mut self, context: egui::Context) {
        let Some(client) = self.client.clone() else {
            log::warn!(target: "companion::app", "device_refresh_skipped outcome=skipped error_kind=not_connected");
            return;
        };
        log::info!(target: "companion::app", "device_refresh_started provider={provider}", provider = self.provider_settings.kind().log_label());
        let (sender, receiver) = mpsc::channel();
        self.device_refresh_result = Some(receiver);
        thread::spawn(move || {
            let result = client.list_targets();
            let _ = sender.send(result);
            context.request_repaint();
        });
    }
    fn poll_device_refresh(&mut self) {
        let Some(receiver) = &self.device_refresh_result else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(devices)) => {
                self.device_refresh_result = None;
                log::info!(
                    target: "companion::app",
                    "device_refresh_succeeded provider={provider} targets={}",
                    devices.len(),
                    provider = self.provider_settings.kind().log_label()
                );
                self.apply_devices(devices);
            }
            Ok(Err(error)) => {
                self.device_refresh_result = None;
                log::warn!(
                    target: "companion::app",
                    "device_refresh_failed provider={provider} error_kind={}",
                    provider_error_kind(&error),
                    provider = self.provider_settings.kind().log_label()
                );
                self.connection_error = Some(error.user_message());
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                log::error!(
                    target: "companion::app",
                    "device_refresh_worker_failed provider={provider} reason=channel_closed",
                    provider = self.provider_settings.kind().log_label()
                );
                self.device_refresh_result = None;
            }
        }
    }

    fn apply_connection_result(&mut self, result: ConnectionResult) {
        match result {
            Ok((client, devices)) => {
                log::info!(
                    target: "companion::app",
                    "connection_test_succeeded provider={provider} targets={}",
                    devices.len(),
                    provider = self.provider_settings.kind().log_label()
                );
                self.client = Some(Arc::new(client));
                self.apply_devices(devices);
                if self.selected_provider == ProviderKind::Local && !self.scanning {
                    self.start_scanning(None);
                }
            }
            Err(error) => {
                log::warn!(
                    target: "companion::app",
                    "connection_test_failed provider={provider} error_kind={}",
                    provider_error_kind(&error),
                    provider = self.provider_settings.kind().log_label()
                );
                self.apply_connection_error(error);
            }
        }
    }
    fn apply_connection_error(&mut self, error: ProviderError) {
        self.client = None;
        self.test_action_result = None;
        self.test_action_status = None;
        self.devices.clear();
        self.selected_device = None;
        self.credential_state = CredentialState::Invalid;
        self.connection_error = Some(error.user_message());
    }
    fn apply_devices(&mut self, devices: Vec<ProviderTarget>) {
        let selected = self
            .preferred_target
            .as_ref()
            .filter(|preferred| devices.iter().any(|device| device.id() == *preferred))
            .cloned()
            .or_else(|| devices.first().map(|device| device.id().clone()));
        self.preferred_target = selected.clone();
        self.selected_device = selected;
        self.devices = devices;
        self.credential_state = CredentialState::Valid;
        self.test_action_status = None;
        self.connection_error = None;
    }
    fn select_device(&mut self, target: TargetId) -> bool {
        if !self.devices.iter().any(|device| device.id() == &target) {
            return false;
        }
        self.selected_device = Some(target.clone());
        self.preferred_target = Some(target);
        self.test_action_status = None;
        true
    }

    fn start_test_action(&mut self, context: egui::Context) {
        let Some(client) = self.client.clone() else {
            log::warn!(target: "companion::app", "test_action_skipped outcome=skipped error_kind=not_connected");
            return;
        };
        let target = self.selected_device().cloned();
        log::info!(
            target: "companion::app",
            "test_action_started outcome=started provider={provider} target={:?}",
            target.as_ref().map(ProviderTarget::id),
            provider = self.provider_settings.kind().log_label()
        );
        let (sender, receiver) = mpsc::channel();
        self.test_action_status = Some(TestActionStatus::Sending);
        self.test_action_result = Some(receiver);
        thread::spawn(move || {
            let result = client.test_action(target.as_ref());
            let _ = sender.send(result);
            context.request_repaint();
        });
    }

    fn poll_test_action(&mut self) {
        let Some(receiver) = &self.test_action_result else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.test_action_result = None;
                self.apply_test_action_result(result);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                log::error!(
                    target: "companion::app",
                    "test_action_worker_failed outcome=failed error_kind=not_connected reason=channel_closed"
                );
                self.test_action_result = None;
                self.apply_test_action_error(ProviderError::NotConnected);
            }
        }
    }

    fn apply_test_action_result(&mut self, result: TestActionResult) {
        match result {
            Ok(()) => {
                log::info!(
                    target: "companion::app",
                    "test_action_succeeded outcome=sent error_kind=none"
                );
                self.test_action_status = Some(TestActionStatus::Sent);
            }
            Err(error) => {
                log::warn!(
                    target: "companion::app",
                    "test_action_failed outcome=failed error_kind={}",
                    provider_error_kind(&error)
                );
                self.apply_test_action_error(error);
            }
        }
    }
    fn apply_test_action_error(&mut self, error: ProviderError) {
        self.test_action_status = Some(TestActionStatus::Failed(format!(
            "Test vibration failed: {}",
            error.user_message()
        )));
    }
    fn copy_action_settings(&mut self, source: TriggerKind, destination: TriggerKind) -> bool {
        if source == destination {
            return false;
        }
        let source_settings = self.triggers.get(source).actions.clone();
        self.triggers
            .get_mut(destination)
            .actions
            .copy_active_from(&source_settings);
        self.copy_feedback = Some(format!(
            "Copied {} {ACTION_KIND_LABEL} settings to {}.",
            source.label(),
            destination.label()
        ));
        true
    }
    fn select_effect(&mut self, kind: TriggerKind) {
        self.selected_effect = kind;
        if self.copy_source == kind {
            self.copy_source = first_copy_source(kind);
        }
        self.copy_feedback = None;
    }

    fn replace_ability_catalog(&mut self, catalog: crate::bridge_listener::AbilityCatalog) {
        self.ability_catalog = catalog
            .abilities
            .into_iter()
            .filter(|ability| ability.ability_slot > 0)
            .map(|ability| (ability.ability_slot, ability.ability_name))
            .collect();
    }
    fn poll_action(&mut self) {
        loop {
            match self.action_result.try_recv() {
                Ok(completion) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    let trigger = &completion.request.trigger;
                    let action_summary = completion.request.resolved.summary();
                    match completion.result {
                        ActionCompletionResult::Skipped { reason } => {
                            log::warn!(
                                target: "companion::app",
                                "action_skipped outcome=skipped error_kind=none trigger={} provider={provider} target={:?} action_kind={ACTION_KIND_LABEL} action_summary={:?} session={} sequence={} reason={}",
                                trigger.kind.label(),
                                completion.request.target.as_ref().map(ProviderTarget::id),
                                action_summary,
                                trigger.session_id,
                                trigger.sequence,
                                reason,
                                provider = self.provider_settings.kind().log_label()
                            );
                            self.action_status = Some(ActionStatus::Skipped {
                                snapshot: ActionSnapshot::from_request(&completion.request),
                                reason: reason.to_owned(),
                            });
                        }
                        ActionCompletionResult::Completed(Ok(())) => {
                            log::info!(
                                target: "companion::app",
                                "action_sent outcome=sent error_kind=none trigger={} provider={provider} target={:?} action_kind={ACTION_KIND_LABEL} action_summary={:?} session={} sequence={}",
                                trigger.kind.label(),
                                completion.request.target.as_ref().map(ProviderTarget::id),
                                action_summary,
                                trigger.session_id,
                                trigger.sequence,
                                provider = self.provider_settings.kind().log_label()
                            );
                            self.action_status = Some(ActionStatus::Sent(completion.request));
                        }
                        ActionCompletionResult::Completed(Err(error)) => {
                            log::warn!(
                                target: "companion::app",
                                "action_failed outcome=failed trigger={} provider={provider} target={:?} action_kind={ACTION_KIND_LABEL} action_summary={:?} session={} sequence={} error_kind={}",
                                trigger.kind.label(),
                                completion.request.target.as_ref().map(ProviderTarget::id),
                                action_summary,
                                trigger.session_id,
                                trigger.sequence,
                                provider_error_kind(&error),
                                provider = self.provider_settings.kind().log_label()
                            );
                            self.action_status = Some(ActionStatus::Failed {
                                request: completion.request,
                                error: error.to_string(),
                            });
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    log::error!(
                        target: "companion::app",
                        "action_worker_channel_failed reason=disconnected in_flight={}",
                        self.action_in_flight
                    );
                    self.action_in_flight = 0;
                    break;
                }
            }
        }
    }

    fn trigger_is_new(&mut self, trigger: &TriggerIdentity) -> bool {
        if let Some((session_id, sequence)) = &self.last_sequence
            && session_id == &trigger.session_id
            && trigger.sequence <= *sequence
        {
            log::debug!(
                target: "companion::app",
                "action_skipped outcome=skipped error_kind=duplicate_or_out_of_order trigger={} provider={provider} target={:?} action_kind={ACTION_KIND_LABEL} session={} sequence={}",
                trigger.kind.label(),
                self.selected_device().map(ProviderTarget::id),
                trigger.session_id,
                trigger.sequence,
                provider = self.provider_settings.kind().log_label()
            );
            return false;
        }
        self.last_sequence = Some((trigger.session_id.clone(), trigger.sequence));
        true
    }

    fn apply_action_enqueue_result(&mut self, request: ActionRequest, result: ActionEnqueueResult) {
        let trigger = &request.trigger;
        match result {
            ActionEnqueueResult::Accepted => {
                self.action_status = Some(ActionStatus::Sending(request.clone()));
                self.action_in_flight = self.action_in_flight.saturating_add(1);
                log::info!(
                    target: "companion::app",
                    "action_queued outcome=queued error_kind=none trigger={} provider={provider} target={:?} action_kind={ACTION_KIND_LABEL} action_summary={:?} session={} sequence={}",
                    trigger.kind.label(),
                    request.target.as_ref().map(ProviderTarget::id),
                    request.resolved.summary(),
                    trigger.session_id,
                    trigger.sequence,
                    provider = self.provider_settings.kind().log_label()
                );
            }
            ActionEnqueueResult::Full => {
                log::warn!(
                    target: "companion::app",
                    "action_skipped outcome=skipped error_kind=none trigger={} provider={provider} target={:?} action_kind={ACTION_KIND_LABEL} action_summary={:?} session={} sequence={} reason=queue_capacity",
                    trigger.kind.label(),
                    request.target.as_ref().map(ProviderTarget::id),
                    request.resolved.summary(),
                    trigger.session_id,
                    trigger.sequence,
                    provider = self.provider_settings.kind().log_label()
                );
                self.action_status = Some(ActionStatus::Skipped {
                    snapshot: ActionSnapshot::from_request(&request),
                    reason: "action queue is full".to_owned(),
                });
            }
            ActionEnqueueResult::Disconnected => {
                log::error!(
                    target: "companion::app",
                    "action_failed outcome=failed trigger={} provider={provider} target={:?} action_kind={ACTION_KIND_LABEL} action_summary={:?} session={} sequence={} reason=worker_unavailable error_kind=worker_unavailable",
                    trigger.kind.label(),
                    request.target.as_ref().map(ProviderTarget::id),
                    request.resolved.summary(),
                    trigger.session_id,
                    trigger.sequence,
                    provider = self.provider_settings.kind().log_label()
                );
                self.action_status = Some(ActionStatus::Failed {
                    request,
                    error: "action worker is unavailable".to_owned(),
                });
            }
        }
    }

    fn queue_trigger_action(&mut self, trigger: TriggerIdentity) {
        if !self.trigger_is_new(&trigger) {
            return;
        }
        let settings = self.triggers.get(trigger.kind);
        if !settings.enabled {
            log::info!(
                target: "companion::app",
                "trigger_disabled trigger={} session_id={:?} sequence={} ability_slot={:?} detection={:?}",
                trigger.kind.label(),
                trigger.session_id,
                trigger.sequence,
                trigger.ability_slot,
                trigger.detection
            );
            return;
        }
        if let (Some(filter), Some(ability_slot)) = (
            self.triggers.ability_filter(trigger.kind),
            trigger.ability_slot,
        ) && !filter.accepts(ability_slot)
        {
            log::info!(
                target: "companion::app",
                "trigger_filtered reason=ability_not_selected trigger={} session_id={:?} sequence={} ability_slot={} detection={:?}",
                trigger.kind.label(),
                trigger.session_id,
                trigger.sequence,
                ability_slot,
                trigger.detection
            );
            return;
        }

        let target = self.selected_device().cloned();
        let resolved = match settings.actions.resolve_checked() {
            Ok(resolved) => resolved,
            Err(error) => {
                log::warn!(
                    target: "companion::app",
                    "action_skipped outcome=skipped trigger={} provider={provider} target={:?} action_kind={ACTION_KIND_LABEL} session={} sequence={} reason=invalid_settings error_kind=invalid_settings validation={}",
                    trigger.kind.label(),
                    target.as_ref().map(ProviderTarget::id),
                    trigger.session_id,
                    trigger.sequence,
                    error,
                    provider = self.provider_settings.kind().log_label()
                );
                self.action_status = Some(ActionStatus::Skipped {
                    snapshot: ActionSnapshot {
                        target,
                        resolved: None,
                        trigger,
                    },
                    reason: format!("invalid action settings: {error}"),
                });
                return;
            }
        };
        let request = ActionRequest {
            target,
            resolved,
            trigger,
            queued_at: Instant::now(),
        };
        let Some(client) = self.client.clone() else {
            log::warn!(
                target: "companion::app",
                "action_skipped outcome=skipped trigger={} provider={provider} target={:?} action_kind={ACTION_KIND_LABEL} action_summary={:?} session={} sequence={} reason=provider_not_connected error_kind=not_connected",
                request.trigger.kind.label(),
                request.target.as_ref().map(ProviderTarget::id),
                request.resolved.summary(),
                request.trigger.session_id,
                request.trigger.sequence,
                provider = self.provider_settings.kind().log_label()
            );
            self.action_status = Some(ActionStatus::Skipped {
                snapshot: ActionSnapshot::from_request(&request),
                reason: "provider is not connected".to_owned(),
            });
            return;
        };
        let enqueue_result = match self.action_sender.try_send(ActionJob {
            client,
            request: request.clone(),
        }) {
            Ok(()) => ActionEnqueueResult::Accepted,
            Err(TrySendError::Full(_job)) => ActionEnqueueResult::Full,
            Err(TrySendError::Disconnected(_job)) => ActionEnqueueResult::Disconnected,
        };
        self.apply_action_enqueue_result(request, enqueue_result);
    }
    fn ensure_bridge_subscription(&mut self) {
        if self.bridge_events.is_none() {
            log::debug!(target: "companion::app", "bridge_subscription_created");
            self.bridge_events = Some(self.bridge_listener.subscribe());
        }
    }
    fn start_log_listener(&mut self, path: PathBuf) -> std::io::Result<()> {
        self.ensure_bridge_subscription();
        self.bridge_listener.start(path)
    }

    fn start_listener_at(
        &mut self,
        path: PathBuf,
        context: LogListenerStartContext,
    ) -> std::io::Result<()> {
        self.listener_action_error = None;
        log::info!(
            target: "companion::app",
            "log_listener_start_requested context={} path={:?}",
            context.label(),
            path
        );
        let result = self.start_log_listener(path.clone());
        match &result {
            Ok(()) => log::info!(
                target: "companion::app",
                "log_listener_started context={} path={:?}",
                context.label(),
                path
            ),
            Err(error) => {
                log::warn!(
                    target: "companion::app",
                    "log_listener_start_failed context={} path={:?} error={:?}",
                    context.label(),
                    path,
                    error
                );
                self.listener_action_error = Some(format!("Could not start listener: {error}"));
            }
        }
        result
    }
    fn poll_bridge_events(&mut self) {
        while let Some(result) = self.bridge_events.as_ref().map(Receiver::try_recv) {
            match result {
                Ok(event) => {
                    match &event {
                        BridgeEvent::HookReady(ready) => log::info!(
                            target: "companion::app",
                            "bridge_hook_ready session_id={:?} client_time_ms={} poll_interval_ms={}",
                            ready.session_id,
                            ready.client_time_ms,
                            ready.poll_interval_ms
                        ),
                        BridgeEvent::AbilityCatalog(catalog) => log::info!(
                            target: "companion::app",
                            "bridge_ability_catalog session_id={:?} client_time_ms={} abilities={}",
                            catalog.session_id,
                            catalog.client_time_ms,
                            catalog.abilities.len()
                        ),
                        BridgeEvent::LocalPlayerDeath(death) => log::info!(
                            target: "companion::app",
                            "bridge_trigger_received trigger=death session_id={:?} sequence={} client_time_ms={} detection={:?}",
                            death.session_id,
                            death.sequence,
                            death.client_time_ms,
                            death.detection
                        ),
                        BridgeEvent::LocalPlayerKill(kill) => log::info!(
                            target: "companion::app",
                            "bridge_trigger_received trigger=kill session_id={:?} sequence={} client_time_ms={} detection={:?} count_before={:?} count_after={:?}",
                            kill.session_id,
                            kill.sequence,
                            kill.client_time_ms,
                            kill.detection,
                            kill.count_before,
                            kill.count_after
                        ),
                        BridgeEvent::LocalPlayerAssist(assist) => log::info!(
                            target: "companion::app",
                            "bridge_trigger_received trigger=assist session_id={:?} sequence={} client_time_ms={} detection={:?} count_before={:?} count_after={:?}",
                            assist.session_id,
                            assist.sequence,
                            assist.client_time_ms,
                            assist.detection,
                            assist.count_before,
                            assist.count_after
                        ),
                        BridgeEvent::AbilityUsed(ability) => log::info!(
                            target: "companion::app",
                            "bridge_trigger_received trigger=ability_use session_id={:?} sequence={} client_time_ms={} ability_slot={} detection={:?} charges_before={:?} charges_after={:?}",
                            ability.session_id,
                            ability.sequence,
                            ability.client_time_ms,
                            ability.ability_slot,
                            ability.detection,
                            ability.charges_before,
                            ability.charges_after
                        ),
                        BridgeEvent::AbilityCooldownReady(ability) => log::info!(
                            target: "companion::app",
                            "bridge_trigger_received trigger=ability_cooldown_ready session_id={:?} sequence={} client_time_ms={} ability_slot={} detection={:?} charges_before={:?} charges_after={:?}",
                            ability.session_id,
                            ability.sequence,
                            ability.client_time_ms,
                            ability.ability_slot,
                            ability.detection,
                            ability.charges_before,
                            ability.charges_after
                        ),
                    }
                    self.last_bridge_event = Some(event.clone());
                    let trigger = match event {
                        BridgeEvent::HookReady(_) => {
                            self.ability_catalog.clear();
                            None
                        }
                        BridgeEvent::AbilityCatalog(catalog) => {
                            self.replace_ability_catalog(catalog);
                            None
                        }
                        BridgeEvent::LocalPlayerDeath(death) => {
                            Some(TriggerIdentity::from_death(death))
                        }
                        BridgeEvent::LocalPlayerKill(kill) => {
                            Some(TriggerIdentity::from_count(TriggerKind::Kill, kill))
                        }
                        BridgeEvent::LocalPlayerAssist(assist) => {
                            Some(TriggerIdentity::from_count(TriggerKind::Assist, assist))
                        }
                        BridgeEvent::AbilityUsed(ability) => Some(TriggerIdentity::from_ability(
                            TriggerKind::AbilityUse,
                            ability,
                        )),
                        BridgeEvent::AbilityCooldownReady(ability) => {
                            Some(TriggerIdentity::from_ability(
                                TriggerKind::AbilityCooldownReady,
                                ability,
                            ))
                        }
                    };
                    if let Some(trigger) = trigger {
                        self.queue_trigger_action(trigger);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    log::warn!(
                        target: "companion::app",
                        "bridge_subscription_failed reason=channel_closed"
                    );
                    self.bridge_events = None;
                    break;
                }
            }
        }
    }

    fn start_listener_from_input(&mut self) {
        self.start_configured_listener(LogListenerStartContext::Manual);
    }

    fn start_configured_listener(&mut self, context: LogListenerStartContext) -> bool {
        let trimmed_path = self.log_path.trim().to_owned();
        if trimmed_path.is_empty() {
            log::warn!(
                target: "companion::app",
                "log_listener_start_skipped context={} reason=empty_path",
                context.label()
            );
            self.listener_action_error =
                Some("Enter a console.log path before starting the listener.".to_owned());
            return false;
        }
        let path = PathBuf::from(trimmed_path);
        self.start_listener_at(path, context).is_ok()
    }

    fn initialize_log_listener<F>(&mut self, detector: F)
    where
        F: FnOnce() -> Result<Detection, DetectionError>,
    {
        let trimmed_path = self.log_path.trim().to_owned();
        if !trimmed_path.is_empty() {
            self.log_path = trimmed_path;
            log::info!(
                target: "companion::app",
                "log_listener_saved_path_selected context=startup path={:?}",
                self.log_path
            );
            self.start_configured_listener(LogListenerStartContext::Startup);
            return;
        }

        self.bridge_listener.stop();
        self.listener_action_error = None;
        log::info!(
            target: "companion::app",
            "log_path_auto_detection_started context=startup"
        );
        self.apply_log_detection_with_context(detector(), LogListenerStartContext::Startup);
    }

    fn auto_detect_log_path(&mut self) {
        self.listener_action_error = None;
        log::info!(
            target: "companion::app",
            "log_path_auto_detection_started context=manual"
        );
        self.apply_log_detection(deadlock_path::detect());
    }

    fn apply_log_detection(&mut self, result: Result<Detection, DetectionError>) {
        self.apply_log_detection_with_context(result, LogListenerStartContext::Manual);
    }

    fn apply_log_detection_with_context(
        &mut self,
        result: Result<Detection, DetectionError>,
        context: LogListenerStartContext,
    ) {
        match result {
            Ok(Detection::Ready { path }) => {
                log::info!(
                    target: "companion::app",
                    "log_path_auto_detection_found context={} path={:?}",
                    context.label(),
                    path
                );
                self.log_path = path.display().to_string();
                self.log_detection_status = match self.start_listener_at(path, context) {
                    Ok(()) => Some(LogDetectionStatus::Found),
                    Err(error) => Some(LogDetectionStatus::Failed(format!(
                        "Deadlock console.log was found, but the listener could not start: {error}"
                    ))),
                };
            }
            Ok(Detection::NotCreated { path }) => {
                log::info!(
                    target: "companion::app",
                    "log_path_auto_detection_not_created context={} path={:?}",
                    context.label(),
                    path
                );
                self.log_path = path.display().to_string();
                self.log_detection_status = match self.start_listener_at(path, context) {
                    Ok(()) => Some(LogDetectionStatus::NotCreated),
                    Err(error) => Some(LogDetectionStatus::Failed(format!(
                        "Deadlock was found, but the listener could not start: {error}"
                    ))),
                };
            }
            Err(error) => {
                log::warn!(
                    target: "companion::app",
                    "log_path_auto_detection_failed context={} error={:?}",
                    context.label(),
                    error
                );
                if context == LogListenerStartContext::Startup {
                    self.log_path.clear();
                    self.bridge_listener.stop();
                }
                self.log_detection_status = Some(LogDetectionStatus::Failed(format!(
                    "Auto-detect failed: {error}"
                )));
            }
        }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        self.poll_test_action();
        self.poll_connection_test();
        self.poll_device_refresh();
        self.poll_scan();
        self.poll_scan_refresh();
        self.poll_action();
        self.poll_bridge_events();
        let busy = self.is_busy();
        self.auto_refresh_scanning_devices(ui.ctx().clone());

        ui.horizontal(|ui| {
            for section in [
                AppSection::Setup,
                AppSection::Effects,
                AppSection::GameConnection,
                AppSection::Donate,
            ] {
                let selected = self.selected_section == section;
                ui.vertical(|ui| {
                    let wing_top = ui.cursor().top();
                    ui.add_space(8.0);
                    let button =
                        egui::Button::new(crate::theme::heading_text(section.label(), 16.0))
                            .fill(if selected {
                                crate::theme::ACCENT_DIM
                            } else {
                                egui::Color32::TRANSPARENT
                            })
                            .stroke(if selected {
                                egui::Stroke::new(1.0, crate::theme::ACCENT)
                            } else {
                                egui::Stroke::NONE
                            });
                    let response = ui.add(button);
                    if selected {
                        let center_x = response.rect.center().x;
                        ui.painter().add(egui::Shape::convex_polygon(
                            vec![
                                egui::pos2(center_x, wing_top),
                                egui::pos2(center_x - 7.0, wing_top + 8.0),
                                egui::pos2(center_x + 7.0, wing_top + 8.0),
                            ],
                            crate::theme::ACCENT,
                            egui::Stroke::NONE,
                        ));
                    }
                    if response.clicked() {
                        self.selected_section = section;
                    }
                });
                ui.add_space(6.0);
            }
        });
        ui.add_space(10.0);

        match self.selected_section {
            AppSection::Setup => self.draw_setup(ui, busy),
            AppSection::Effects => self.draw_effects(ui, busy),
            AppSection::GameConnection => self.draw_game_connection(ui),
            AppSection::Donate => Self::draw_donate(ui),
        }

        let listener_status = self.bridge_listener.status();
        if listener_status.phase != ListenerPhase::Stopped || self.action_in_progress() {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }

    fn draw_setup(&mut self, ui: &mut Ui, busy: bool) {
        self.draw_connection_card(ui, busy);
        ui.add_space(10.0);
        self.draw_toy_card(ui, busy);
    }

    fn draw_connection_card(&mut self, ui: &mut Ui, busy: bool) {
        crate::theme::card(ui).show(ui, |ui| {
            ui.horizontal(|ui| {
                crate::theme::colored_text_nudged_down(
                    ui,
                    egui_phosphor::regular::HEART,
                    crate::theme::ACCENT,
                    4.0,
                );
                ui.label(crate::theme::heading_text("Provider", 19.0));
            });
            ui.add_space(4.0);
            ui.label("Choose how the companion talks to your toys.");
            ui.add_space(14.0);

            ui.add_enabled_ui(!busy, |ui| {
                egui::ComboBox::from_id_salt("provider")
                    .selected_text(self.selected_provider.label())
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for kind in [
                            ProviderKind::Lovense,
                            ProviderKind::Local,
                            ProviderKind::Intiface,
                        ] {
                            ui.selectable_value(&mut self.selected_provider, kind, kind.label());
                        }
                    });
            });
            if self.selected_provider != self.provider_settings.kind() {
                self.select_provider(self.selected_provider);
            }
            ui.add_space(14.0);

            match self.selected_provider {
                ProviderKind::Lovense => self.draw_lovense_fields(ui, busy),
                ProviderKind::Local => self.draw_local_fields(ui, busy),
                ProviderKind::Intiface => self.draw_intiface_fields(ui, busy),
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                crate::theme::badge(
                    ui,
                    self.credential_state.label(),
                    self.credential_state.tone(),
                );
                if let Some(error) = &self.connection_error {
                    crate::theme::badge(ui, error, crate::theme::BadgeTone::Danger);
                }
            });
        });
    }

    fn draw_lovense_fields(&mut self, ui: &mut Ui, busy: bool) {
        ui.label(crate::theme::heading_text("Lovense connection", 19.0));
        ui.add_space(4.0);
        ui.label(
            "This connects the companion to the Lovense Remote app on your PC, which talks to your toy. Most people won't need to change any settings here.",
        );
        ui.add_space(14.0);

        ui.label(egui::RichText::new("1. Open the Lovense Remote app").strong());
        ui.small("Make sure your toy already shows as connected in it.");
        ui.add_space(10.0);
        ui.label(egui::RichText::new("2. Turn on Game Mode").strong());
        ui.small(
            "Find it in the Remote app's menu. This lets other apps, like this one, send vibration commands to your toy.",
        );
        ui.add_space(10.0);
        ui.label(egui::RichText::new("3. Test the connection").strong());
        ui.small(
            "Click the button below \u{2014} you're done once it says \u{201c}Connected\u{201d}.",
        );
        ui.add_space(14.0);

        let can_test = self.credentials_present() && !busy;
        if ui
            .add_enabled(
                can_test,
                egui::Button::new(egui::RichText::new("Test connection").size(15.0))
                    .min_size(egui::vec2(ui.available_width(), 34.0)),
            )
            .clicked()
        {
            self.start_connection_test(ui.ctx().clone());
        }
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(8.0);
        let mut credentials_changed = false;
        egui::CollapsingHeader::new("Advanced: custom domain / port")
            .id_salt("lovense-advanced")
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.small(
                    "Only needed if your toy connects through Lovense Remote on your phone instead of this PC. The app's Game Mode screen shows the domain and port to enter here.",
                );
                ui.add_space(8.0);
                ui.add_enabled_ui(!busy, |ui| {
                    let setup = self.provider_settings.lovense_mut();
                    credentials_changed |= text_input(ui, "Domain", &mut setup.domain, false);
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("HTTP port");
                        credentials_changed |= ui
                            .add(egui::DragValue::new(&mut setup.http_port))
                            .changed();
                    });
                });
            });
        if credentials_changed {
            self.reset_connection();
        }
    }

    fn draw_local_fields(&mut self, ui: &mut Ui, busy: bool) {
        ui.label(crate::theme::heading_text(
            "Local (Embedded Intiface)",
            19.0,
        ));
        ui.add_space(4.0);
        ui.label(
            "Runs a built-in buttplug engine inside the companion, so no separate Intiface Central is needed. Scanning starts automatically once the engine is running — keep it on while your toy is discoverable; toys usually show up within a few seconds.",
        );
        ui.add_space(8.0);
        ui.small(
            "Lovense over direct Bluetooth: close the Lovense Remote app (or disconnect the toy in it) first — a toy already holding a Bluetooth connection can't be found by a second app. Alternatively, leave Game Mode on with internet access and the engine will pick the toy up through the Lovense Connect relay instead. Also check: Bluetooth is on, the toy is unpaired from the OS Bluetooth settings, and it is awake/pairing.",
        );
        ui.add_space(14.0);

        let connected = self.client.is_some() && !busy;
        let engine_label = if connected {
            "Stop / disconnect embedded engine"
        } else {
            "Start embedded engine"
        };
        if ui
            .add_enabled(
                !busy,
                egui::Button::new(egui::RichText::new(engine_label).size(15.0))
                    .min_size(egui::vec2(ui.available_width(), 34.0)),
            )
            .clicked()
        {
            if connected {
                self.reset_connection();
            } else {
                self.start_connection_test(ui.ctx().clone());
            }
        }
        ui.add_space(10.0);

        let toggle_enabled = connected;
        let toggle_label = if self.scanning {
            "Stop scanning"
        } else {
            "Start scanning for toys"
        };
        if ui
            .add_enabled(
                toggle_enabled,
                egui::Button::new(egui::RichText::new(toggle_label).size(15.0))
                    .min_size(egui::vec2(ui.available_width(), 34.0)),
            )
            .clicked()
        {
            if self.scanning {
                self.stop_scanning(Some(ui.ctx().clone()));
            } else {
                self.start_scanning(Some(ui.ctx().clone()));
            }
        }
    }

    fn draw_intiface_fields(&mut self, ui: &mut Ui, busy: bool) {
        ui.label(crate::theme::heading_text("Intiface Central", 19.0));
        ui.add_space(4.0);
        ui.label(
            "Connect to an externally running Intiface Central (or any buttplug WebSocket server). Start the server in Intiface Central, then enter its address below and test the connection.",
        );
        ui.add_space(14.0);

        ui.add_enabled_ui(!busy, |ui| {
            let setup = self.provider_settings.intiface_mut();
            let changed = text_input(ui, "WebSocket URL", &mut setup.websocket_url, false);
            if changed {
                self.reset_connection();
            }
        });
        ui.add_space(10.0);

        let can_test = self.credentials_present() && !busy;
        if ui
            .add_enabled(
                can_test,
                egui::Button::new(egui::RichText::new("Test connection").size(15.0))
                    .min_size(egui::vec2(ui.available_width(), 34.0)),
            )
            .clicked()
        {
            self.start_connection_test(ui.ctx().clone());
        }
    }

    fn draw_toy_card(&mut self, ui: &mut Ui, busy: bool) {
        crate::theme::card(ui).show(ui, |ui| {
            ui.horizontal(|ui| {
                crate::theme::colored_text_nudged_down(
                    ui,
                    egui_phosphor::regular::VIBRATE,
                    crate::theme::ACCENT,
                    4.0,
                );
                ui.label(crate::theme::heading_text("Toy", 19.0));
            });
            ui.add_space(4.0);
            ui.label("Pick which toy reacts to in-game events, once connected above.");
            ui.add_space(10.0);
            let selection_enabled = !self.devices.is_empty() && !busy;
            let selected_name = self
                .selected_device()
                .map(|device| device.name().to_owned())
                .unwrap_or_else(|| {
                    if self.devices.is_empty() {
                        "No toys found yet".to_owned()
                    } else {
                        "All connected toys".to_owned()
                    }
                });
            let mut selection_changed = false;
            let mut selected_device = self.selected_device.clone();
            ui.horizontal(|ui| {
                let can_refresh = self.client.is_some() && !busy;
                let refresh_width = 34.0;
                let spacing = ui.spacing().item_spacing.x;
                ui.add_enabled_ui(selection_enabled, |ui| {
                    egui::ComboBox::from_id_salt("device")
                        .selected_text(selected_name.as_str())
                        .width(ui.available_width() - refresh_width - spacing)
                        .show_ui(ui, |ui| {
                            for device in &self.devices {
                                selection_changed |= ui
                                    .selectable_value(
                                        &mut selected_device,
                                        Some(device.id().clone()),
                                        device.name(),
                                    )
                                    .changed();
                            }
                        });
                });
                if ui
                    .add_enabled(
                        can_refresh,
                        egui::Button::new(egui_phosphor::regular::ARROWS_CLOCKWISE)
                            .min_size(egui::vec2(refresh_width, 0.0)),
                    )
                    .on_hover_text("Refresh the toy list")
                    .clicked()
                {
                    self.start_device_refresh(ui.ctx().clone());
                }
            });
            if selection_changed && let Some(target) = selected_device {
                self.select_device(target);
            }
            ui.add_space(4.0);
            if self.devices.is_empty() {
                ui.small("Just connected your toy? Click the refresh button above \u{2014} the list only updates when asked.");
            } else {
                ui.small("Leave it on \u{201c}All connected toys\u{201d} unless you specifically want just one to react.");
            }
            if self.devices.is_empty() {
                if let Some(error) = &self.connection_error {
                    ui.add_space(4.0);
                    crate::theme::badge(ui, error, crate::theme::BadgeTone::Danger);
                } else {
                    ui.add_space(4.0);
                    ui.small("Still nothing after refreshing? Double-check \u{201c}Game Mode\u{201d} is toggled on in the Lovense Remote app. A toy can show connected there without being visible to Game Mode.");
                }
            }
            ui.add_space(10.0);
            let can_test = self.client.is_some() && !busy;
            if ui
                .add_enabled(
                    can_test,
                    egui::Button::new(egui::RichText::new("Send test vibration").size(15.0))
                        .min_size(egui::vec2(ui.available_width(), 34.0)),
                )
                .clicked()
            {
                self.start_test_action(ui.ctx().clone());
            }
            if let Some(status) = &self.test_action_status {
                ui.add_space(8.0);
                crate::theme::badge(ui, status.label(), status.tone());
            }
        });
    }

    fn draw_effects(&mut self, ui: &mut Ui, busy: bool) {
        ui.columns(2, |columns| {
            self.draw_trigger_list(&mut columns[0], busy);
            self.draw_effect_editor(&mut columns[1], busy);
        });
    }

    fn draw_trigger_list(&mut self, ui: &mut Ui, busy: bool) {
        ui.label(crate::theme::heading_text("Triggers", 26.0));
        crate::theme::flourish(ui);
        ui.small(format!(
            "Each trigger has its own {ACTION_KIND_LABEL} settings."
        ));
        ui.add_space(6.0);
        for kind in [
            TriggerKind::Death,
            TriggerKind::Kill,
            TriggerKind::Assist,
            TriggerKind::AbilityUse,
            TriggerKind::AbilityCooldownReady,
        ] {
            let summary = self.triggers.get(kind).actions.summary();
            let enabled = self.triggers.get(kind).enabled;
            let selected = self.selected_effect == kind;
            let frame = if selected {
                crate::theme::card_selected(ui)
            } else {
                crate::theme::card(ui)
            };
            let mut configure_clicked = false;
            let mut controls_rect = egui::Rect::NOTHING;
            let card = frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    crate::theme::icon_badge(ui, trigger_icon(kind), enabled);
                    // Add controls right-to-left first so the text vertical gets
                    // only the space between the icon and the controls, not the
                    // full remaining card width.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        configure_clicked |= ui.button("Configure").clicked();
                        ui.add_enabled_ui(!busy, |ui| {
                            toggle_button(ui, &mut self.triggers.get_mut(kind).enabled);
                        });
                        // Capture the controls-only rect before adding the text block.
                        controls_rect = ui.min_rect();
                        // Text fills the remaining space to the left of the controls.
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                            ui.vertical(|ui| {
                                ui.label(crate::theme::heading_text(
                                    trigger_display_label(kind),
                                    18.0,
                                ));
                                if enabled {
                                    ui.colored_label(
                                        crate::theme::ACCENT_BRIGHT,
                                        format!(
                                            "{} mode",
                                            self.triggers.get(kind).actions.mode.label()
                                        ),
                                    );
                                }
                                ui.horizontal(|ui| {
                                    ui.colored_label(
                                        crate::theme::ACCENT,
                                        egui_phosphor::regular::HEART,
                                    );
                                    ui.small(summary.clone());
                                });
                            });
                        });
                    });
                });
            });
            // The toggle/Configure controls sit inside the card, so the
            // whole-card click sense below is restricted to the area left of
            // them. Otherwise it sits on top in interaction order and
            // swallows clicks meant for the toggle button.
            let mut click_rect = card.response.rect;
            if controls_rect.is_positive() {
                click_rect.max.x = controls_rect.min.x;
            }
            let card_id = ui.id().with(("trigger_card", trigger_display_label(kind)));
            let card_response = ui.interact(click_rect, card_id, egui::Sense::click());
            if card_response.clicked() {
                configure_clicked = true;
            }
            if card_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if configure_clicked {
                self.select_effect(kind);
            }
            ui.add_space(6.0);
        }
        ui.add_space(4.0);
        crate::theme::flourish(ui);
    }

    /// Reserves the same vertical space as the trigger list's heading +
    /// flourish + caption (drawn transparently, so its height matches
    /// exactly rather than being guessed), so the effect editor's card
    /// starts level with the first trigger card next to it.
    fn draw_effect_editor_header_spacer(ui: &mut Ui) {
        ui.scope(|ui| {
            ui.visuals_mut().override_text_color = Some(egui::Color32::TRANSPARENT);
            ui.label(crate::theme::heading_text("Triggers", 26.0));
        });
        ui.add_space(14.0); // matches theme::flourish's fixed height
        ui.scope(|ui| {
            ui.visuals_mut().override_text_color = Some(egui::Color32::TRANSPARENT);
            ui.small("spacer");
        });
        ui.add_space(6.0);
    }

    fn draw_effect_editor(&mut self, ui: &mut Ui, busy: bool) {
        let destination = self.selected_effect;
        Self::draw_effect_editor_header_spacer(ui);
        crate::theme::card(ui).show(ui, |ui| {
            ui.horizontal(|ui| {
                crate::theme::icon_badge_sized(ui, trigger_icon(destination), true, 56.0);
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.label(crate::theme::heading_text(
                        format!("{} effect", trigger_display_label(destination)),
                        22.0,
                    ));
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.add_space(3.0);
                                ui.label(crate::theme::heading_text("Trigger", 16.0));
                            });
                            toggle_button(ui, &mut self.triggers.get_mut(destination).enabled);
                        });
                    });
                });
            });
            if matches!(
                destination,
                TriggerKind::AbilityUse | TriggerKind::AbilityCooldownReady
            ) {
                self.draw_ability_filter(ui, destination, busy);
                if destination == TriggerKind::AbilityCooldownReady {
                    ui.small("Cooldown ready includes a normal cooldown finishing and a charged ability restoring a charge.");
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                crate::theme::label_nudged_down(
                    ui,
                    &format!("Copy {ACTION_KIND_LABEL} settings from"),
                    8.0,
                );
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ComboBox::from_id_salt("copy-action-source")
                        .selected_text(trigger_display_label(self.copy_source))
                        .show_ui(ui, |ui| {
                            for source in [
                                TriggerKind::Death,
                                TriggerKind::Kill,
                                TriggerKind::Assist,
                                TriggerKind::AbilityUse,
                                TriggerKind::AbilityCooldownReady,
                            ] {
                                if source != destination {
                                    ui.selectable_value(
                                        &mut self.copy_source,
                                        source,
                                        trigger_display_label(source),
                                    );
                                }
                            }
                        });
                });
                ui.add_enabled_ui(!busy && self.copy_source != destination, |ui| {
                    let label = format!("{} Copy", egui_phosphor::regular::HEART);
                    if ui.button(label).clicked() {
                        self.copy_action_settings(self.copy_source, destination);
                    }
                });
            });
            if let Some(feedback) = &self.copy_feedback {
                status_line(ui, feedback, [0.30, 0.78, 0.42, 1.0]);
            }
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            ui.add_enabled_ui(!busy, |ui| {
                let trigger = self.triggers.get_mut(destination);
                crate::action_ui::draw_vibrate_settings_editor(ui, &mut trigger.actions);
            });
        });
    }

    fn draw_ability_filter(&mut self, ui: &mut Ui, kind: TriggerKind, busy: bool) {
        let mut slots: BTreeSet<u32> = (1..=4).collect();
        slots.extend(self.ability_catalog.keys().copied());
        let slots: Vec<u32> = slots.into_iter().collect();
        let names = self.ability_catalog.clone();
        let filter = self
            .triggers
            .ability_filter_mut(kind)
            .expect("ability trigger has an ability filter");

        ui.add_space(6.0);
        ui.label("Abilities");
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                if ui.button("All").clicked() {
                    *filter = AbilityFilter::All;
                }
                if ui.button("None").clicked() {
                    *filter = AbilityFilter::Selected(BTreeSet::new());
                }
            });
            ui.horizontal_wrapped(|ui| {
                for slot in &slots {
                    let mut selected = filter.accepts(*slot);
                    let label = names
                        .get(slot)
                        .and_then(Option::as_deref)
                        .map(|name| format!("Slot {slot}: {name}"))
                        .unwrap_or_else(|| format!("Slot {slot}"));
                    if ui.checkbox(&mut selected, label).changed() {
                        if matches!(&*filter, AbilityFilter::All) {
                            let selected_slots: BTreeSet<u32> = slots
                                .iter()
                                .copied()
                                .filter(|candidate| *candidate != *slot)
                                .collect();
                            *filter = AbilityFilter::Selected(selected_slots);
                        } else if let AbilityFilter::Selected(selected_slots) = &mut *filter {
                            if selected {
                                selected_slots.insert(*slot);
                            } else {
                                selected_slots.remove(slot);
                            }
                        }
                    }
                }
            });
        });
        if matches!(&*filter, AbilityFilter::Selected(slots) if slots.is_empty()) {
            status_line(
                ui,
                "No abilities are selected; this trigger will not send an action.",
                [0.92, 0.68, 0.22, 1.0],
            );
        }
        if self.ability_catalog.is_empty() {
            ui.small("Using numbered slots until the game reports ability names.");
        }
    }

    fn draw_donate(ui: &mut Ui) {
        crate::theme::card(ui).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(6.0);
                ui.colored_label(
                    crate::theme::ACCENT,
                    egui::RichText::new(egui_phosphor::regular::HEART).size(40.0),
                );
                ui.add_space(6.0);
                ui.label(crate::theme::heading_text("Enjoying Lovelock Companion?", 24.0));
                ui.small("If it's brought a little extra fun to your matches, consider supporting development on Ko-fi.");
                ui.add_space(12.0);
                let button = egui::Button::new(
                    egui::RichText::new(format!(
                        "{} Support on Ko-fi",
                        egui_phosphor::regular::HEART
                    ))
                    .strong(),
                )
                .fill(crate::theme::ACCENT);
                if ui.add(button).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(KOFI_URL));
                }
                ui.add_space(6.0);
                ui.hyperlink_to(KOFI_URL, KOFI_URL);
            });
        });

        ui.add_space(10.0);

        crate::theme::card(ui).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(6.0);
                ui.colored_label(
                    crate::theme::ACCENT,
                    egui::RichText::new(egui_phosphor::regular::SPARKLE).size(28.0),
                );
                ui.add_space(6.0);
                ui.label(crate::theme::heading_text("Shoutout to volc", 20.0));
                ui.small("The original creator of DeadlockShock, which this companion is built on. Go check them out!");
                ui.add_space(10.0);
                let button = egui::Button::new(
                    egui::RichText::new(format!(
                        "{} Visit volc on GameBanana",
                        egui_phosphor::regular::SPARKLE
                    ))
                    .strong(),
                )
                .fill(crate::theme::ACCENT);
                if ui.add(button).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(VOLC_PROFILE_URL));
                }
                ui.add_space(6.0);
                ui.hyperlink_to(VOLC_PROFILE_URL, VOLC_PROFILE_URL);
            });
        });
    }

    fn draw_game_connection(&mut self, ui: &mut Ui) {
        crate::theme::card(ui).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(
                    crate::theme::ACCENT,
                    egui_phosphor::regular::PLUGS_CONNECTED,
                );
                ui.label(crate::theme::heading_text("Game connection", 19.0));
            });
            ui.small("Deadlock must be launched with -condebug so it writes console.log.");
            ui.add_space(4.0);
            text_input(ui, "Log path", &mut self.log_path, false);
            ui.horizontal(|ui| {
                let spacing = ui.spacing().item_spacing.x;
                let button_size = egui::vec2(
                    (ui.available_width() - spacing) * 0.5,
                    ui.spacing().interact_size.y,
                );
                if ui
                    .add_sized(button_size, egui::Button::new("Auto-detect"))
                    .clicked()
                {
                    self.auto_detect_log_path();
                }
                if ui
                    .add_sized(button_size, egui::Button::new("Start/Restart listener"))
                    .clicked()
                {
                    self.start_listener_from_input();
                }
            });
            if let Some(status) = &self.log_detection_status {
                ui.add_space(4.0);
                crate::theme::badge(ui, status.label(), status.tone());
            }
            if let Some(error) = &self.listener_action_error {
                status_line(ui, error, [0.92, 0.32, 0.28, 1.0]);
            }
            ui.add_space(6.0);
            let listener_status = self.bridge_listener.status();
            draw_listener_status(ui, &listener_status, self.last_bridge_event.as_ref());
            ui.label(format!(
                "Current ability catalogue: {} slot(s).",
                self.ability_catalog.len()
            ));
            if let Some(status) = &self.action_status {
                let label = status.label();
                status_line(ui, &label, status.color());
            } else {
                ui.label("Last action delivery: none since startup.");
            }
        });
    }
}

pub struct CompanionApp {
    pub state: AppState,
    persistence: Persistence,
    reset_confirmation: bool,
    menu_error: Option<String>,
    version_check: VersionCheckOwner,
    version_warnings: WarningSelection,
    log_store: LogStore,
    logs_window_open: bool,
    logs_cached_revision: u64,
    logs_cached_text: String,
    logo_texture: Option<egui::TextureHandle>,
    credits_window_open: bool,
}

impl CompanionApp {
    pub fn load() -> Self {
        Self::load_with_store(LogStore::new())
    }

    fn load_with_store(log_store: LogStore) -> Self {
        match default_state_path() {
            Ok(path) => {
                Self::load_from_path_with_detector_and_store(path, deadlock_path::detect, log_store)
            }
            Err(error) => {
                log::warn!(
                    target: "companion::app",
                    "settings_load_unavailable error={:?}",
                    error
                );
                let (persistence, state) = Persistence::unavailable(error);
                Self::from_persisted_state(persistence, state, deadlock_path::detect, log_store)
            }
        }
    }

    pub fn load_with_context(context: egui::Context, log_store: LogStore) -> Self {
        let mut app = Self::load_with_store(log_store);
        app.version_check = VersionCheckOwner::new(&context);
        // The Lovense connection itself is never persisted (only the setup
        // that reaches it is), so without this the toy has to be manually
        // reconnected every time the app is opened, even though everything
        // needed to do so automatically is already saved.
        if app.state.credentials_present() {
            log::info!(target: "companion::app", "startup_connection_attempt provider={provider}", provider = app.state.provider_settings.kind().log_label());
            app.state.start_connection_test(context);
        }
        app
    }

    fn load_from_path_with_detector<F>(path: PathBuf, detector: F) -> Self
    where
        F: FnOnce() -> Result<Detection, DetectionError>,
    {
        Self::load_from_path_with_detector_and_store(path, detector, LogStore::new())
    }

    fn load_from_path_with_detector_and_store<F>(
        path: PathBuf,
        detector: F,
        log_store: LogStore,
    ) -> Self
    where
        F: FnOnce() -> Result<Detection, DetectionError>,
    {
        let (persistence, state) = Persistence::open(path);
        Self::from_persisted_state(persistence, state, detector, log_store)
    }

    fn from_persisted_state<F>(
        persistence: Persistence,
        persisted_state: PersistedState,
        detector: F,
        log_store: LogStore,
    ) -> Self
    where
        F: FnOnce() -> Result<Detection, DetectionError>,
    {
        let mut state = persisted_state.restore_app();
        state.initialize_log_listener(detector);
        Self {
            state,
            persistence,
            reset_confirmation: false,
            menu_error: None,
            version_check: VersionCheckOwner::with_client(LATEST_RELEASE_URL, None),
            version_warnings: WarningSelection::default(),
            log_store,
            logs_window_open: false,
            logs_cached_revision: 0,
            logs_cached_text: String::new(),
            logo_texture: None,
            credits_window_open: false,
        }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        self.version_check.poll();
        let listener_status = self.state.bridge_listener.status();
        let remote = match &self.version_check.state {
            VersionCheckState::Current { latest }
            | VersionCheckState::UpdateAvailable { latest } => Some(latest),
            _ => None,
        };
        self.version_warnings =
            select_warnings(&app_version(), &listener_status.mod_version, remote);

        draw_resize_borders(ui.ctx());
        self.draw_title_bar(ui);

        if self.persistence.load_warning().is_some()
            || self.persistence.save_error().is_some()
            || self.menu_error.is_some()
            || self.has_update_warning()
        {
            egui::Panel::top("notices")
                .frame(egui::Frame::NONE.fill(crate::theme::BASE).inner_margin(8.0))
                .show(ui, |ui| {
                    if let Some(warning) = self.persistence.load_warning() {
                        status_line(ui, warning, [0.92, 0.68, 0.22, 1.0]);
                    }
                    if let Some(error) = self.persistence.save_error() {
                        status_line(ui, error, [0.92, 0.32, 0.28, 1.0]);
                    }
                    if let Some(error) = &self.menu_error {
                        status_line(ui, error, [0.92, 0.32, 0.28, 1.0]);
                    }
                    self.draw_update_panel(ui);
                });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(crate::theme::BASE))
            .show(ui, |ui| {
                crate::theme::paint_dotted_background(ui);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Frame::NONE.inner_margin(16.0).show(ui, |ui| {
                        self.state.draw(ui);
                    });
                });
            });
        let ctx = ui.ctx().clone();
        self.draw_reset_confirmation(&ctx);
        self.draw_logs_window(&ctx);
        self.draw_credits_window(&ctx);

        if let Some(delay) = self
            .persistence
            .observe(PersistedState::from_app(&self.state), Instant::now())
        {
            ctx.request_repaint_after(delay);
        }
        if self.version_check.is_checking() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn has_update_warning(&self) -> bool {
        self.version_warnings.companion_outdated.is_some()
            || self.version_warnings.mod_outdated.is_some()
            || self.version_warnings.mod_legacy
            || self.version_warnings.mod_invalid
    }

    fn draw_title_bar(&mut self, ui: &mut Ui) {
        let bar_height = 44.0;
        egui::Panel::top("title_bar")
            .exact_size(bar_height)
            .frame(
                egui::Frame::NONE
                    .fill(crate::theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(ui, |ui| {
                let ctx = ui.ctx().clone();
                ui.horizontal_centered(|ui| {
                    let drag_response = ui.interact(
                        ui.max_rect(),
                        ui.id().with("title_bar_drag"),
                        egui::Sense::click_and_drag(),
                    );
                    if drag_response.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if drag_response.double_clicked() {
                        let maximized =
                            ctx.input(|input| input.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }

                    if self.logo_texture.is_none() {
                        let image = image::load_from_memory_with_format(
                            include_bytes!("../assets/logo.png"),
                            image::ImageFormat::Png,
                        )
                        .expect("logo.png must be a valid PNG")
                        .into_rgba8();
                        let size = [image.width() as usize, image.height() as usize];
                        self.logo_texture = Some(ui.ctx().load_texture(
                            "lovelock-logo",
                            egui::ColorImage::from_rgba_unmultiplied(size, &image.into_raw()),
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                    if let Some(logo) = &self.logo_texture {
                        ui.add(
                            egui::Image::new(egui::load::SizedTexture::from_handle(logo))
                                .fit_to_exact_size(egui::vec2(28.0, 28.0)),
                        );
                    }
                    ui.add_space(8.0);
                    ui.label(crate::theme::heading_text("Lovelock Companion", 20.0));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new(egui_phosphor::regular::X).frame(false))
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui
                            .add(egui::Button::new(egui_phosphor::regular::SQUARE).frame(false))
                            .clicked()
                        {
                            let maximized =
                                ctx.input(|input| input.viewport().maximized.unwrap_or(false));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        }
                        if ui
                            .add(egui::Button::new(egui_phosphor::regular::MINUS).frame(false))
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        ui.add_space(10.0);
                        let gear_response = ui.add(
                            egui::Button::new(egui_phosphor::regular::GEAR)
                                .corner_radius(egui::CornerRadius::same(255))
                                .stroke(egui::Stroke::new(1.0, crate::theme::STROKE)),
                        );
                        egui::Popup::menu(&gear_response).show(|ui| {
                            ui.set_min_width(220.0);
                            self.draw_menu_contents(ui);
                        });
                        ui.add_space(10.0);
                        crate::theme::badge(
                            ui,
                            &format!("v{}", app_version()),
                            crate::theme::BadgeTone::Success,
                        );
                    });
                });
            });
    }
    fn draw_logs_window(&mut self, ctx: &egui::Context) {
        if !self.logs_window_open {
            return;
        }

        let revision = self.log_store.revision();
        if revision != self.logs_cached_revision {
            let snapshot: LogSnapshot = self.log_store.snapshot();
            self.logs_cached_revision = snapshot.revision;
            self.logs_cached_text = snapshot.text;
        }
        ctx.request_repaint_after(Duration::from_millis(250));

        let mut open = self.logs_window_open;
        egui::Window::new("Logs").open(&mut open).show(ctx, |ui| {
            if ui.button("Copy all").clicked() {
                ctx.copy_text(self.logs_cached_text.clone());
            }
            if self.logs_cached_text.is_empty() {
                ui.label("No log records have been captured yet.");
                return;
            }
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&self.logs_cached_text).monospace())
                            .selectable(true),
                    );
                });
        });
        self.logs_window_open = open;
    }

    fn draw_credits_window(&mut self, ctx: &egui::Context) {
        if !self.credits_window_open {
            return;
        }

        let mut open = true;
        egui::Window::new("Credits")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    ui.colored_label(
                        crate::theme::ACCENT,
                        egui::RichText::new(egui_phosphor::regular::HEART).size(32.0),
                    );
                    ui.add_space(8.0);
                    ui.label(crate::theme::heading_text("volc", 18.0));
                    ui.label(
                        "A HUGE THANK YOU to volc for DeadlockShock, the mod this companion is built on.",
                    );
                    ui.add_space(12.0);
                    ui.label(crate::theme::heading_text("KaufkinNova", 18.0));
                    ui.label("A HUGE THANK YOU to KaufkinNova for the idea and for sponsoring this mod.");
                    ui.add_space(4.0);
                });
            });
        self.credits_window_open = open;
    }

    fn draw_update_panel(&self, ui: &mut Ui) {
        let has_warning = self.version_warnings.companion_outdated.is_some()
            || self.version_warnings.mod_outdated.is_some()
            || self.version_warnings.mod_legacy
            || self.version_warnings.mod_invalid;
        if !has_warning {
            return;
        }
        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(86, 64, 22))
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.strong("Updates available");
                if let Some(target) = &self.version_warnings.companion_outdated {
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "Companion {} is older than {}.",
                            app_version(),
                            target
                        ));
                        ui.hyperlink_to("Download companion", COMPANION_RELEASE_URL);
                    });
                }
                if let Some((installed, target)) = &self.version_warnings.mod_outdated {
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "DeadlockShock mod {} is older than {}.",
                            installed, target
                        ));
                        ui.hyperlink_to("Update mod", MOD_RELEASE_URL);
                    });
                } else if self.version_warnings.mod_legacy {
                    ui.horizontal(|ui| {
                        ui.label("The last observed DeadlockShock mod predates version reporting.");
                        ui.hyperlink_to("Update mod", MOD_RELEASE_URL);
                    });
                } else if self.version_warnings.mod_invalid {
                    ui.horizontal(|ui| {
                        ui.label("The last observed DeadlockShock mod reported an invalid version; reinstall the latest mod.");
                        ui.hyperlink_to("Update mod", MOD_RELEASE_URL);
                    });
                }
            });
    }
    pub fn flush_pending(&mut self) {
        log::info!(target: "companion::app", "settings_flush_boundary reason=application_exit");
        let result = self
            .persistence
            .flush(PersistedState::from_app(&self.state));
        if result.is_err() {
            log::warn!(target: "companion::app", "settings_flush_boundary outcome=failed");
        }
    }
    fn draw_menu_contents(&mut self, ui: &mut Ui) {
        let reset_available = !self.state.is_busy();
        ui.label(format!("Companion version: {}", app_version()));
        let mod_label = match &self.state.bridge_listener.status().mod_version {
            ModVersionObservation::Unknown => "unknown".to_owned(),
            ModVersionObservation::Legacy => "legacy (no version reporting)".to_owned(),
            ModVersionObservation::Invalid => "invalid".to_owned(),
            ModVersionObservation::Reported(version) => format!("last observed {version}"),
        };
        ui.label(format!("Mod version: {mod_label}"));
        match &self.version_check.state {
            VersionCheckState::Checking => ui.label("Latest stable: checking…"),
            VersionCheckState::Current { latest } => {
                ui.label(format!("Latest stable: {latest} (current)"))
            }
            VersionCheckState::UpdateAvailable { latest } => {
                ui.label(format!("Latest stable: {latest} (update available)"))
            }
            VersionCheckState::Unavailable { reason } => {
                ui.label(format!("Latest stable: unavailable ({reason})"))
            }
        };
        let checking = self.version_check.is_checking();
        if ui
            .add_enabled(!checking, egui::Button::new("Check for updates"))
            .clicked()
        {
            self.version_check.start(ui.ctx().clone());
        }
        ui.separator();
        if ui.button("Open config folder").clicked() {
            log::info!(target: "companion::app", "config_folder_open_requested");
            self.menu_error = self.persistence.open_config_directory().err();
            if let Some(error) = &self.menu_error {
                log::warn!(
                    target: "companion::app",
                    "config_folder_open_failed error={:?}",
                    error
                );
            }
        }
        if ui.button("Show logs").clicked() {
            self.logs_window_open = true;
            ui.close();
        }
        if ui.button("Credits").clicked() {
            self.credits_window_open = true;
            ui.close();
        }
        ui.separator();
        let response = ui.add_enabled(reset_available, egui::Button::new("Reset saved state…"));
        if response.clicked() {
            self.reset_confirmation = true;
        }
        if !reset_available {
            response.on_disabled_hover_text(
                "Wait for connection, test action, and action work to finish before resetting.",
            );
        }
    }

    fn draw_reset_confirmation(&mut self, ctx: &egui::Context) {
        if !self.reset_confirmation {
            return;
        }

        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;
        let reset_available = !self.state.is_busy();
        egui::Window::new("Reset saved state?")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    "This clears saved provider setup, target preference, trigger action settings, and log path.",
                );
                ui.label("Any active log listener will be stopped.");
                if !reset_available {
                    status_line(
                        ui,
                        "Wait for connection, test action, and action work to finish.",
                        [0.92, 0.68, 0.22, 1.0],
                    );
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add_enabled(reset_available, egui::Button::new("Reset"))
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        self.reset_confirmation = open && !cancel;
        if confirm && self.reset_and_save() {
            self.reset_confirmation = false;
        }
    }

    fn reset_and_save(&mut self) -> bool {
        log::info!(target: "companion::app", "settings_reset_requested");
        if !self.state.reset_saved_state() {
            log::warn!(
                target: "companion::app",
                "settings_reset_outcome outcome=skipped"
            );
            return false;
        }
        let result = self
            .persistence
            .save_reset_now(PersistedState::from_app(&self.state));
        log::info!(
            target: "companion::app",
            "settings_reset_outcome outcome=applied saved={}",
            result.is_ok()
        );
        true
    }
}
fn trigger_display_label(kind: TriggerKind) -> &'static str {
    match kind {
        TriggerKind::Death => "Death",
        TriggerKind::Kill => "Kill",
        TriggerKind::Assist => "Assist",
        TriggerKind::AbilityUse => "Ability use",
        TriggerKind::AbilityCooldownReady => "Cooldown ready",
    }
}

fn trigger_icon(kind: TriggerKind) -> &'static str {
    match kind {
        TriggerKind::Death => egui_phosphor::regular::SKULL,
        TriggerKind::Kill => egui_phosphor::regular::SWORD,
        TriggerKind::Assist => egui_phosphor::regular::HANDSHAKE,
        TriggerKind::AbilityUse => egui_phosphor::regular::MAGIC_WAND,
        TriggerKind::AbilityCooldownReady => egui_phosphor::regular::HOURGLASS_SIMPLE,
    }
}

fn first_copy_source(destination: TriggerKind) -> TriggerKind {
    match destination {
        TriggerKind::Death => TriggerKind::AbilityUse,
        TriggerKind::Kill
        | TriggerKind::Assist
        | TriggerKind::AbilityUse
        | TriggerKind::AbilityCooldownReady => TriggerKind::Death,
    }
}

/// The window has no OS decorations (custom title bar instead), so there is
/// no native edge-drag resize affordance. This paints invisible drag strips
/// along the screen edges/corners that issue `BeginResize` to the backend.
fn draw_resize_borders(ctx: &egui::Context) {
    let screen = ctx.input(|i| i.viewport_rect());
    if !screen.is_positive() {
        return;
    }

    let border = 6.0;
    let corner = 14.0;
    let zones: [(egui::Rect, egui::CursorIcon, egui::ResizeDirection); 8] = [
        (
            egui::Rect::from_min_size(screen.left_top(), egui::vec2(corner, corner)),
            egui::CursorIcon::ResizeNorthWest,
            egui::ResizeDirection::NorthWest,
        ),
        (
            egui::Rect::from_min_size(
                egui::pos2(screen.right() - corner, screen.top()),
                egui::vec2(corner, corner),
            ),
            egui::CursorIcon::ResizeNorthEast,
            egui::ResizeDirection::NorthEast,
        ),
        (
            egui::Rect::from_min_size(
                egui::pos2(screen.left(), screen.bottom() - corner),
                egui::vec2(corner, corner),
            ),
            egui::CursorIcon::ResizeSouthWest,
            egui::ResizeDirection::SouthWest,
        ),
        (
            egui::Rect::from_min_size(
                egui::pos2(screen.right() - corner, screen.bottom() - corner),
                egui::vec2(corner, corner),
            ),
            egui::CursorIcon::ResizeSouthEast,
            egui::ResizeDirection::SouthEast,
        ),
        (
            egui::Rect::from_min_size(
                egui::pos2(screen.left(), screen.top() + corner),
                egui::vec2(border, (screen.height() - 2.0 * corner).max(0.0)),
            ),
            egui::CursorIcon::ResizeWest,
            egui::ResizeDirection::West,
        ),
        (
            egui::Rect::from_min_size(
                egui::pos2(screen.right() - border, screen.top() + corner),
                egui::vec2(border, (screen.height() - 2.0 * corner).max(0.0)),
            ),
            egui::CursorIcon::ResizeEast,
            egui::ResizeDirection::East,
        ),
        (
            egui::Rect::from_min_size(
                egui::pos2(screen.left() + corner, screen.top()),
                egui::vec2((screen.width() - 2.0 * corner).max(0.0), border),
            ),
            egui::CursorIcon::ResizeNorth,
            egui::ResizeDirection::North,
        ),
        (
            egui::Rect::from_min_size(
                egui::pos2(screen.left() + corner, screen.bottom() - border),
                egui::vec2((screen.width() - 2.0 * corner).max(0.0), border),
            ),
            egui::CursorIcon::ResizeSouth,
            egui::ResizeDirection::South,
        ),
    ];

    egui::Area::new(egui::Id::new("resize_borders"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            for (index, (rect, cursor, direction)) in zones.into_iter().enumerate() {
                if !rect.is_positive() {
                    continue;
                }
                let id = ui.id().with(("resize_zone", index));
                let response = ui.interact(rect, id, egui::Sense::drag());
                if response.hovered() || response.dragged() {
                    ui.ctx().set_cursor_icon(cursor);
                }
                if response.drag_started() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
                }
            }
        });
}

fn toggle_button(ui: &mut Ui, value: &mut bool) {
    let label = if *value { "On" } else { "Off" };
    let (fill, text_color) = if *value {
        (crate::theme::ACCENT, egui::Color32::WHITE)
    } else {
        (crate::theme::CARD_RAISED, crate::theme::TEXT_DIM)
    };
    let button = egui::Button::new(egui::RichText::new(label).strong().color(text_color))
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(255))
        .min_size(egui::vec2(52.0, 0.0));
    if ui.add(button).clicked() {
        *value = !*value;
    }
}

fn draw_listener_status(ui: &mut Ui, status: &ListenerStatus, last_event: Option<&BridgeEvent>) {
    let (phase_label, phase_tone) = match status.phase {
        ListenerPhase::Stopped => (
            "Listener stopped.".to_owned(),
            crate::theme::BadgeTone::Neutral,
        ),
        ListenerPhase::WaitingForFile => (
            "Listener waiting for console.log to be created.".to_owned(),
            crate::theme::BadgeTone::Warning,
        ),
        ListenerPhase::Listening => (
            "Listener is monitoring console.log.".to_owned(),
            crate::theme::BadgeTone::Success,
        ),
        ListenerPhase::Failed => (
            format!(
                "Listener failed: {}",
                status.current_error.as_deref().unwrap_or("unknown error")
            ),
            crate::theme::BadgeTone::Danger,
        ),
    };
    if let Some(path) = &status.configured_path {
        ui.label(format!("Configured listener path: {}", path.display()));
    }
    crate::theme::badge(ui, &phase_label, phase_tone);
    let activity = status
        .last_activity_at
        .map(|at| format!("Last log activity: {} ago.", format_duration(at.elapsed())))
        .unwrap_or_else(|| "Last log activity: none since listener start.".to_owned());
    ui.label(activity);
    let event = match (last_event, status.last_event_at) {
        (Some(event), Some(at)) => format!(
            "Last bridge event: {} ({} ago).",
            bridge_event_description(event),
            format_duration(at.elapsed())
        ),
        _ => "Last bridge event: none since listener start.".to_owned(),
    };
    ui.label(event);
}
fn bridge_event_description(event: &BridgeEvent) -> String {
    let ability_description = |name: &str, ability: &AbilityTrigger| {
        let ability_name = ability
            .ability_name
            .as_deref()
            .map(|name| format!(" ({name})"))
            .unwrap_or_default();
        let charges = match (ability.charges_before, ability.charges_after) {
            (Some(before), Some(after)) => format!(", charges {before}→{after}"),
            _ => String::new(),
        };
        format!(
            "{name}, slot {}{ability_name}, detection {}{charges}",
            ability.ability_slot, ability.detection
        )
    };
    let count_description = |count: &CountTrigger| {
        let counts = match (count.count_before, count.count_after) {
            (Some(before), Some(after)) => format!(", count {before}→{after}"),
            _ => String::new(),
        };
        format!("detection {}{counts}", count.detection)
    };
    match event {
        BridgeEvent::HookReady(_) | BridgeEvent::LocalPlayerDeath(_) => {
            event.event_name().to_owned()
        }
        BridgeEvent::AbilityCatalog(catalog) => {
            format!("ability_catalog, {} slot(s)", catalog.abilities.len())
        }
        BridgeEvent::LocalPlayerKill(count) => {
            format!("local_player_kill, {}", count_description(count))
        }
        BridgeEvent::LocalPlayerAssist(count) => {
            format!("local_player_assist, {}", count_description(count))
        }
        BridgeEvent::AbilityUsed(ability) => ability_description("ability_used", ability),
        BridgeEvent::AbilityCooldownReady(ability) => {
            ability_description("ability_cooldown_ready", ability)
        }
    }
}
fn format_duration(duration: Duration) -> String {
    if duration.as_secs() >= 60 {
        format!("{}m {}s", duration.as_secs() / 60, duration.as_secs() % 60)
    } else {
        format!("{:.1}s", duration.as_secs_f32())
    }
}
fn input_background() -> Color32 {
    Color32::from_rgb(38, 38, 42)
}
fn text_input(ui: &mut Ui, label: &str, value: &mut String, password: bool) -> bool {
    ui.label(label);
    ui.add(
        TextEdit::singleline(value)
            .password(password)
            .desired_width(f32::INFINITY)
            .background_color(input_background()),
    )
    .changed()
}
fn status_line(ui: &mut Ui, value: &str, color: [f32; 4]) {
    ui.colored_label(to_color(color), value);
}
fn to_color(color: [f32; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (color[0].clamp(0.0, 1.0) * 255.0) as u8,
        (color[1].clamp(0.0, 1.0) * 255.0) as u8,
        (color[2].clamp(0.0, 1.0) * 255.0) as u8,
        (color[3].clamp(0.0, 1.0) * 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::VibrateMode;
    use crate::logging::CapturingWriter;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use std::io::Write;

    fn trigger(kind: TriggerKind, session_id: &str, sequence: u64) -> TriggerIdentity {
        let is_ability = matches!(
            kind,
            TriggerKind::AbilityUse | TriggerKind::AbilityCooldownReady
        );
        TriggerIdentity {
            kind,
            session_id: session_id.to_owned(),
            sequence,
            client_time_ms: sequence,
            detection: "test".to_owned(),
            ability_slot: is_ability.then_some(2),
            ability_name: is_ability.then(|| "Test Ability".to_owned()),
            charges_before: None,
            charges_after: None,
        }
    }

    fn resolved(strength: u8, duration_secs: u32) -> ResolvedVibrateAction {
        ResolvedVibrateAction {
            strength,
            duration_secs,
        }
    }

    #[test]
    fn action_resolution_is_an_immutable_snapshot() {
        let mut settings = VibrateActionSettings::default();
        settings.mode = VibrateMode::Fixed;
        settings.fixed.strength = 14.0;
        settings.fixed.duration_seconds = 3.0;
        let mut rng = StdRng::seed_from_u64(4);
        let snapshot = settings.resolve_with(&mut rng).unwrap();
        settings.fixed.strength = 20.0;
        assert_eq!(settings.fixed.strength, 20.0);
        assert_eq!(snapshot.strength, 14);
        assert_eq!(snapshot.duration_secs, 3);
    }

    #[test]
    fn invalid_fixed_action_settings_are_skipped_without_fabricating_an_action() {
        let mut state = AppState::default();
        state.triggers.death.actions.mode = VibrateMode::Fixed;
        state.triggers.death.actions.fixed.strength = 21.0;
        state.queue_trigger_action(trigger(TriggerKind::Death, "session", 1));
        let Some(ActionStatus::Skipped { snapshot, reason }) = state.action_status else {
            panic!("invalid settings should be skipped");
        };
        assert!(snapshot.resolved.is_none());
        assert!(reason.contains("invalid action settings"));
    }

    #[test]
    fn copy_transfers_only_active_action_settings() {
        let mut state = AppState::default();
        state.triggers.death.enabled = false;
        state.triggers.death.actions.mode = VibrateMode::Fixed;
        state.triggers.death.actions.fixed.strength = 15.0;
        state.triggers.ability_use.trigger.enabled = true;
        state.triggers.ability_use.ability_filter = AbilityFilter::Selected(BTreeSet::from([2]));
        assert!(state.copy_action_settings(TriggerKind::Death, TriggerKind::AbilityUse));
        assert_eq!(
            state.triggers.ability_use.trigger.actions,
            state.triggers.death.actions
        );
        assert!(state.triggers.ability_use.trigger.enabled);
        assert_eq!(
            state.triggers.ability_use.ability_filter,
            AbilityFilter::Selected(BTreeSet::from([2]))
        );
    }

    #[test]
    fn action_queue_preserves_capacity_and_expiry() {
        let (sender, receiver) = mpsc::sync_channel(ACTION_QUEUE_CAPACITY);
        for value in 0..ACTION_QUEUE_CAPACITY {
            assert!(sender.try_send(value).is_ok());
        }
        assert!(matches!(
            sender.try_send(ACTION_QUEUE_CAPACITY),
            Err(TrySendError::Full(_))
        ));
        drop(receiver);
        assert!(matches!(
            sender.try_send(0),
            Err(TrySendError::Disconnected(_))
        ));
        let queued_at = Instant::now();
        assert!(action_job_expired_at(
            queued_at,
            queued_at + MAX_ACTION_QUEUE_AGE
        ));
    }

    #[test]
    fn queue_outcomes_use_generic_action_statuses() {
        let request = ActionRequest {
            target: None,
            resolved: resolved(15, 3),
            trigger: trigger(TriggerKind::Death, "session", 1),
            queued_at: Instant::now(),
        };
        let mut full = AppState::default();
        full.apply_action_enqueue_result(request.clone(), ActionEnqueueResult::Full);
        assert!(matches!(
            full.action_status,
            Some(ActionStatus::Skipped { reason, .. }) if reason == "action queue is full"
        ));
        let mut disconnected = AppState::default();
        disconnected
            .apply_action_enqueue_result(request.clone(), ActionEnqueueResult::Disconnected);
        assert!(matches!(
            disconnected.action_status,
            Some(ActionStatus::Failed { error, .. }) if error == "action worker is unavailable"
        ));
        let mut accepted = AppState::default();
        accepted.apply_action_enqueue_result(request, ActionEnqueueResult::Accepted);
        assert_eq!(accepted.action_in_flight, 1);
        assert!(matches!(
            accepted.action_status,
            Some(ActionStatus::Sending(_))
        ));
    }

    #[test]
    fn global_sequence_watermark_is_preserved_across_trigger_kinds() {
        let mut state = AppState::default();
        state.queue_trigger_action(trigger(TriggerKind::AbilityUse, "first", 4));
        assert_eq!(state.last_sequence, Some(("first".to_owned(), 4)));
        state.queue_trigger_action(trigger(TriggerKind::Death, "first", 3));
        state.queue_trigger_action(trigger(TriggerKind::Death, "first", 5));
        assert_eq!(state.last_sequence, Some(("first".to_owned(), 5)));
    }

    #[test]
    fn kill_and_assist_triggers_queue_independently_of_death() {
        let mut state = AppState {
            devices: vec![ProviderTarget::new("toy-1", "hub")],
            selected_device: Some("toy-1".to_owned()),
            ..AppState::default()
        };
        state.triggers.kill.enabled = true;
        state.triggers.assist.enabled = true;
        state.queue_trigger_action(trigger(TriggerKind::Kill, "session", 1));
        assert!(matches!(
            state.action_status,
            Some(ActionStatus::Skipped { ref reason, .. }) if reason == "provider is not connected"
        ));
        state.queue_trigger_action(trigger(TriggerKind::Assist, "session", 2));
        assert!(matches!(
            state.action_status,
            Some(ActionStatus::Skipped { ref reason, .. }) if reason == "provider is not connected"
        ));
    }

    #[test]
    fn disabled_kill_trigger_is_ignored() {
        let mut state = AppState::default();
        assert!(!state.triggers.kill.enabled);
        state.queue_trigger_action(trigger(TriggerKind::Kill, "session", 1));
        assert!(state.action_status.is_none());
    }

    #[test]
    fn action_status_contains_provider_target_and_trigger_details() {
        let status = ActionStatus::Sent(ActionRequest {
            target: Some(ProviderTarget::new("group".to_owned(), "group")),
            resolved: resolved(15, 3),
            trigger: {
                let mut trigger = trigger(TriggerKind::AbilityCooldownReady, "session", 9);
                trigger.detection = "charge_restored".to_owned();
                trigger.charges_before = Some(1);
                trigger.charges_after = Some(2);
                trigger
            },
            queued_at: Instant::now(),
        });
        let label = status.label();
        assert!(label.contains("group"));
        assert!(label.contains("15/20 for 3 s"));
        assert!(label.contains("ability cooldown ready slot 2"));
        assert!(label.contains("charge_restored"));
        assert!(label.contains("charges 1→2"));
    }

    #[test]
    fn reset_clears_durable_banks_and_runtime_action_state() {
        let mut state = AppState::default();
        state.provider_settings.lovense_mut().domain = "custom.lan".into();
        state.triggers.death.actions.mode = VibrateMode::Fixed;
        assert!(state.reset_saved_state());
        assert_eq!(state.provider_settings, ProviderSettings::default());
        assert!(state.runtime_trigger_and_action_state_is_clear());
    }
    #[test]
    fn preferred_target_is_reconciled_against_fresh_targets() {
        let preferred: TargetId = "toy-2".to_owned();
        let mut state = AppState {
            preferred_target: Some(preferred.clone()),
            ..AppState::default()
        };
        state.apply_devices(vec![
            ProviderTarget::new("toy-1", "Alpha"),
            ProviderTarget::new(preferred.clone(), "Beta"),
        ]);
        assert_eq!(state.selected_device, Some(preferred.clone()));
        assert_eq!(
            state.selected_device().map(ProviderTarget::name),
            Some("Beta")
        );
        state.reset_connection();
        assert!(state.selected_device.is_none());
        assert_eq!(state.preferred_target, Some(preferred.clone()));
        state.apply_connection_result(Err(ProviderError::NotConnected));
        assert_eq!(state.preferred_target, Some(preferred));
        state.apply_devices(vec![ProviderTarget::new("toy-3", "Gamma")]);
        assert_eq!(state.selected_device, Some("toy-3".to_owned()));
        assert!(!state.select_device("toy-99".to_owned()));
    }

    #[test]
    fn failed_connection_clears_stale_live_targets_but_preserves_preference() {
        let mut state = AppState {
            devices: vec![ProviderTarget::new("toy-1", "hub")],
            selected_device: Some("toy-1".to_owned()),
            preferred_target: Some("toy-1".to_owned()),
            ..AppState::default()
        };
        state.apply_connection_result(Err(ProviderError::NotConnected));
        assert!(state.devices.is_empty());
        assert!(state.selected_device.is_none());
        assert_eq!(state.preferred_target, Some("toy-1".to_owned()));
        assert_eq!(state.credential_state, CredentialState::Invalid);
    }

    #[test]
    fn test_action_status_reports_success_and_failure() {
        let mut state = AppState::default();
        state.apply_test_action_result(Ok(()));
        assert_eq!(state.test_action_status, Some(TestActionStatus::Sent));
        state.apply_test_action_result(Err(ProviderError::NotConnected));
        assert!(matches!(
            state.test_action_status,
            Some(TestActionStatus::Failed(_))
        ));
    }

    #[test]
    fn all_effect_editors_render_both_vibrate_modes() {
        for kind in [
            TriggerKind::Death,
            TriggerKind::Kill,
            TriggerKind::Assist,
            TriggerKind::AbilityUse,
            TriggerKind::AbilityCooldownReady,
        ] {
            for mode in [VibrateMode::Interval, VibrateMode::Fixed] {
                let context = egui::Context::default();
                crate::theme::install_fonts(&context);
                let mut state = AppState::default();
                state.selected_section = AppSection::Effects;
                state.selected_effect = kind;
                state.triggers.get_mut(kind).actions.mode = mode;
                if matches!(
                    kind,
                    TriggerKind::AbilityUse | TriggerKind::AbilityCooldownReady
                ) {
                    state
                        .ability_catalog
                        .insert(1, Some("Power Slash".to_owned()));
                }
                let output = context.run_ui(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| state.draw(ui));
                });
                assert!(!output.shapes.is_empty());
            }
        }
    }

    #[test]
    fn detection_status_updates_path_and_guidance() {
        let path = PathBuf::from("/steam/Deadlock/game/citadel/console.log");
        let mut state = AppState::default();
        state.apply_log_detection(Ok(Detection::Ready { path: path.clone() }));
        assert_eq!(state.log_path, path.display().to_string());
        assert_eq!(state.log_detection_status, Some(LogDetectionStatus::Found));
        state.apply_log_detection(Ok(Detection::NotCreated { path }));
        assert!(
            state
                .log_detection_status
                .as_ref()
                .expect("status")
                .label()
                .contains("-condebug")
        );
        state.log_path = "/manual/console.log".into();
        state.apply_log_detection(Err(DetectionError::DeadlockNotInstalled));
        assert_eq!(state.log_path, "/manual/console.log");
    }

    #[test]
    fn manual_listener_restart_uses_configured_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("console.log");
        std::fs::write(&path, b"").unwrap();
        let mut state = AppState {
            log_path: path.display().to_string(),
            ..AppState::default()
        };

        state.start_listener_from_input();

        let status = state.bridge_listener.status();
        assert_eq!(status.configured_path, Some(path));
        assert_eq!(status.phase, ListenerPhase::Listening);
        assert!(state.bridge_events.is_some());
        assert!(state.listener_action_error.is_none());
    }

    #[test]
    fn startup_saved_path_starts_without_detection() {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().join("state.json");
        let log_path = directory.path().join("console.log");
        std::fs::write(&log_path, b"").unwrap();
        {
            let mut first_launch =
                CompanionApp::load_from_path_with_detector(state_path.clone(), || {
                    Err(DetectionError::DeadlockNotInstalled)
                });
            first_launch.state.log_path = format!("  {}  ", log_path.display());
            first_launch.flush_pending();
            assert!(first_launch.persistence.save_error().is_none());
        }

        let app = CompanionApp::load_from_path_with_detector(
            state_path,
            || -> Result<Detection, DetectionError> {
                panic!("saved path startup must not detect");
            },
        );

        let status = app.state.bridge_listener.status();
        assert_eq!(app.state.log_path, log_path.display().to_string());
        assert!(app.state.bridge_events.is_some());
        assert_eq!(status.configured_path, Some(log_path));
        assert_eq!(status.phase, ListenerPhase::Listening);
        assert!(app.state.listener_action_error.is_none());
    }

    #[test]
    fn startup_saved_missing_path_waits_without_detection_or_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("console.log");
        let mut state = AppState {
            log_path: path.display().to_string(),
            ..AppState::default()
        };

        state.initialize_log_listener(|| -> Result<Detection, DetectionError> {
            panic!("saved path startup must not detect");
        });

        let status = state.bridge_listener.status();
        assert_eq!(status.configured_path, Some(path));
        assert_eq!(status.phase, ListenerPhase::WaitingForFile);
        assert!(state.listener_action_error.is_none());
    }

    #[test]
    fn startup_empty_state_ready_detection_starts_listener() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("console.log");
        std::fs::write(&path, b"").unwrap();
        let mut state = AppState::default();

        state.initialize_log_listener(|| Ok(Detection::Ready { path: path.clone() }));

        let status = state.bridge_listener.status();
        assert_eq!(state.log_path, path.display().to_string());
        assert_eq!(state.log_detection_status, Some(LogDetectionStatus::Found));
        assert_eq!(status.configured_path, Some(path));
        assert_eq!(status.phase, ListenerPhase::Listening);
    }

    #[test]
    fn startup_empty_state_not_created_detection_waits_with_guidance() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("console.log");
        let mut state = AppState::default();

        state.initialize_log_listener(|| Ok(Detection::NotCreated { path: path.clone() }));

        let status = state.bridge_listener.status();
        assert_eq!(state.log_path, path.display().to_string());
        assert_eq!(
            state.log_detection_status,
            Some(LogDetectionStatus::NotCreated)
        );
        assert!(
            state
                .log_detection_status
                .as_ref()
                .expect("not-created status")
                .label()
                .contains("-condebug")
        );
        assert_eq!(status.configured_path, Some(path));
        assert_eq!(status.phase, ListenerPhase::WaitingForFile);
    }

    #[test]
    fn startup_detection_failure_is_non_fatal_and_stays_stopped() {
        let mut state = AppState::default();

        state.initialize_log_listener(|| Err(DetectionError::DeadlockNotInstalled));

        assert!(state.log_path.is_empty());
        assert_eq!(state.bridge_listener.status().phase, ListenerPhase::Stopped);
        assert!(state.bridge_events.is_none());
        assert!(state.listener_action_error.is_none());
        assert!(matches!(
            state.log_detection_status,
            Some(LogDetectionStatus::Failed(_))
        ));
    }

    #[test]
    fn startup_reset_stops_listener_without_second_detection() {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().join("state.json");
        let log_path = directory.path().join("console.log");
        let detection_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let detector_calls = Arc::clone(&detection_calls);
        let mut app = CompanionApp::load_from_path_with_detector(state_path, move || {
            detector_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Detection::NotCreated {
                path: log_path.clone(),
            })
        });

        assert_eq!(detection_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            app.state.bridge_listener.status().phase,
            ListenerPhase::WaitingForFile
        );
        assert!(app.reset_and_save());
        assert_eq!(
            app.state.bridge_listener.status().phase,
            ListenerPhase::Stopped
        );
        assert_eq!(detection_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn expired_completion_is_skipped_and_decrements_in_flight() {
        let request = ActionRequest {
            target: None,
            resolved: resolved(15, 3),
            trigger: trigger(TriggerKind::Death, "session", 1),
            queued_at: Instant::now(),
        };
        let (sender, receiver) = mpsc::channel();
        let mut state = AppState {
            action_result: receiver,
            action_in_flight: 1,
            ..AppState::default()
        };
        sender
            .send(ActionCompletion {
                request,
                result: ActionCompletionResult::Skipped { reason: "expired" },
            })
            .unwrap();
        state.poll_action();
        assert_eq!(state.action_in_flight, 0);
        assert!(matches!(
            state.action_status,
            Some(ActionStatus::Skipped { reason, .. }) if reason == "expired"
        ));
    }

    #[test]
    fn actionable_events_share_global_watermark_and_disabled_filtering_advances_it() {
        let mut state = AppState::default();
        state.queue_trigger_action(trigger(TriggerKind::AbilityUse, "first", 4));
        assert_eq!(state.last_sequence, Some(("first".to_owned(), 4)));
        state.queue_trigger_action(trigger(TriggerKind::Death, "first", 3));
        state.triggers.ability_use.trigger.enabled = true;
        state.triggers.ability_use.ability_filter = AbilityFilter::Selected(BTreeSet::new());
        state.queue_trigger_action(trigger(TriggerKind::AbilityUse, "first", 5));
        assert_eq!(state.last_sequence, Some(("first".to_owned(), 5)));
        state.triggers.ability_use.ability_filter = AbilityFilter::All;
        state.queue_trigger_action(trigger(TriggerKind::AbilityUse, "first", 5));
        assert!(state.action_status.is_none());
        state.queue_trigger_action(trigger(TriggerKind::AbilityUse, "first", 6));
        assert_eq!(state.last_sequence, Some(("first".to_owned(), 6)));
    }

    #[test]
    fn parsed_enabled_ability_event_reaches_action_queue_path() {
        let event = crate::bridge_listener::parse_bridge_record(
            "[DEADLOCK_DEATH_HOOK]{\"schema\":1,\"event\":\"ability_cooldown_ready\",\"session_id\":\"session\",\"client_time_ms\":7,\"sequence\":3,\"ability_slot\":2,\"ability_name\":\"Bookwyrm\",\"detection\":\"charge_restored\",\"charges_before\":1,\"charges_after\":2}",
        )
        .expect("valid ability event");
        let (sender, receiver) = mpsc::channel();
        let mut state = AppState {
            bridge_events: Some(receiver),
            ..AppState::default()
        };
        state.triggers.ability_cooldown_ready.trigger.enabled = true;
        sender.send(event).unwrap();
        state.poll_bridge_events();
        assert_eq!(state.last_sequence, Some(("session".to_owned(), 3)));
        assert!(matches!(
            state.action_status,
            Some(ActionStatus::Skipped { .. })
        ));
        assert_eq!(state.action_in_flight, 0);
    }

    #[test]
    fn hook_ready_and_catalog_events_do_not_advance_actionable_state() {
        let (sender, receiver) = mpsc::channel();
        let mut state = AppState {
            bridge_events: Some(receiver),
            ..AppState::default()
        };
        state
            .ability_catalog
            .insert(1, Some("Stale name".to_owned()));
        sender
            .send(BridgeEvent::HookReady(crate::bridge_listener::HookReady {
                schema: 1,
                session_id: "session".to_owned(),
                client_time_ms: 1,
                poll_interval_ms: 100,
            }))
            .unwrap();
        sender
            .send(BridgeEvent::AbilityCatalog(
                crate::bridge_listener::AbilityCatalog {
                    schema: 1,
                    session_id: "session".to_owned(),
                    client_time_ms: 2,
                    abilities: vec![crate::bridge_listener::AbilityCatalogEntry {
                        ability_slot: 2,
                        ability_name: Some("Replacement".to_owned()),
                    }],
                },
            ))
            .unwrap();
        state.poll_bridge_events();
        assert!(state.last_sequence.is_none());
        assert_eq!(
            state.ability_catalog.get(&2),
            Some(&Some("Replacement".to_owned()))
        );
    }

    #[test]
    fn trigger_routing_resolves_selected_profile_before_status() {
        let mut state = AppState::default();
        state.triggers.ability_use.trigger.enabled = true;
        state.triggers.ability_use.trigger.actions.mode = VibrateMode::Fixed;
        state.triggers.ability_use.trigger.actions.fixed.strength = 17.0;
        state
            .triggers
            .ability_use
            .trigger
            .actions
            .fixed
            .duration_seconds = 5.0;
        state.queue_trigger_action(trigger(TriggerKind::AbilityUse, "session", 1));
        let Some(status) = state.action_status.clone() else {
            panic!("enabled trigger should record missing-provider status");
        };
        assert_eq!(status.snapshot().resolved, Some(resolved(17, 5)));
        state.triggers.ability_use.trigger.actions.fixed.strength = 5.0;
        assert_eq!(status.snapshot().resolved, Some(resolved(17, 5)));
    }

    #[test]
    fn ability_filters_cover_all_selected_empty_and_unknown_slots() {
        assert!(AbilityFilter::All.accepts(1));
        assert!(AbilityFilter::All.accepts(999));
        let selected = AbilityFilter::Selected(BTreeSet::from([2, 5]));
        assert!(!selected.accepts(1));
        assert!(selected.accepts(2));
        assert!(!selected.accepts(999));
        assert!(!AbilityFilter::Selected(BTreeSet::new()).accepts(2));
    }

    #[test]
    fn selecting_effect_updates_editor_and_copy_source() {
        let mut state = AppState {
            selected_effect: TriggerKind::Death,
            copy_source: TriggerKind::AbilityUse,
            copy_feedback: Some("old confirmation".to_owned()),
            ..AppState::default()
        };
        state.select_effect(TriggerKind::AbilityUse);
        assert_eq!(state.selected_effect, TriggerKind::AbilityUse);
        assert_eq!(state.copy_source, TriggerKind::Death);
        assert!(state.copy_feedback.is_none());
    }

    #[test]
    fn reset_is_blocked_by_each_in_flight_work_kind() {
        let mut connection_busy = AppState {
            provider_settings: ProviderSettings::Lovense(crate::provider::LovenseSetup {
                domain: "keep.lan".to_owned(),
                ..Default::default()
            }),
            ..AppState::default()
        };
        let (_sender, receiver) = mpsc::channel();
        connection_busy.connection_result = Some(receiver);
        assert!(!connection_busy.reset_saved_state());
        assert_eq!(
            connection_busy.provider_settings.lovense().domain,
            "keep.lan"
        );
        let mut test_busy = AppState::default();
        let (_sender, receiver) = mpsc::channel();
        test_busy.test_action_result = Some(receiver);
        assert!(!test_busy.reset_saved_state());
        let mut action_busy = AppState {
            action_in_flight: 1,
            ..AppState::default()
        };
        assert!(!action_busy.reset_saved_state());
        let mut refresh_busy = AppState::default();
        let (_sender, receiver) = mpsc::channel();
        refresh_busy.device_refresh_result = Some(receiver);
        assert!(refresh_busy.is_busy());
        assert!(!refresh_busy.reset_saved_state());
    }

    #[test]
    fn device_refresh_replaces_stale_toy_list_without_reconnecting() {
        let mut state = AppState::default();
        state.devices = vec![ProviderTarget::new("stale-toy", "Stale")];
        state.selected_device = Some("stale-toy".to_owned());
        let (sender, receiver) = mpsc::channel();
        state.device_refresh_result = Some(receiver);
        sender
            .send(Ok(vec![ProviderTarget::new("fresh-toy", "Fresh")]))
            .unwrap();
        state.poll_device_refresh();
        assert!(state.device_refresh_result.is_none());
        assert_eq!(
            state.devices,
            vec![ProviderTarget::new("fresh-toy", "Fresh")]
        );
        assert_eq!(state.selected_device, Some("fresh-toy".to_owned()));
    }

    #[test]
    fn reset_clears_listener_runtime_and_writes_durable_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let mut app = CompanionApp::load_from_path_with_detector(path.clone(), || {
            Err(DetectionError::DeadlockNotInstalled)
        });
        app.state.provider_settings.lovense_mut().domain = "custom.lan".to_owned();
        app.state.preferred_target = Some("group".to_owned());
        app.state.triggers.death.actions.mode = VibrateMode::Fixed;
        app.state.triggers.death.actions.fixed.strength = 18.0;
        app.state.log_path = directory.path().join("console.log").display().to_string();
        let _ = app
            .state
            .start_log_listener(PathBuf::from(&app.state.log_path));
        assert!(app.reset_and_save());
        assert_eq!(
            PersistedState::from_app(&app.state),
            PersistedState::default()
        );
        assert!(!app.state.listener_is_running());
        assert!(app.state.runtime_trigger_and_action_state_is_clear());
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            serde_json::to_string_pretty(&PersistedState::default()).unwrap() + "\n"
        );
    }

    #[test]
    fn persistence_aware_app_renders_with_injected_state_path() {
        let directory = tempfile::tempdir().unwrap();
        let mut app =
            CompanionApp::load_from_path_with_detector(directory.path().join("state.json"), || {
                Err(DetectionError::DeadlockNotInstalled)
            });
        let context = egui::Context::default();
        crate::theme::install_fonts(&context);
        let output = context.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw(ui));
        });
        assert!(!output.shapes.is_empty());
    }
    #[test]
    fn injected_logs_render_and_reopen_without_persisted_state_changes() {
        let directory = tempfile::tempdir().unwrap();
        let store = LogStore::new();
        let mut writer = CapturingWriter::new(store.clone(), Vec::<u8>::new());
        writer.write_all(b"startup_record\nlive_record\n").unwrap();
        let mut app = CompanionApp::load_from_path_with_detector_and_store(
            directory.path().join("state.json"),
            || Err(DetectionError::DeadlockNotInstalled),
            store,
        );
        let persisted = PersistedState::from_app(&app.state);
        let context = egui::Context::default();
        crate::theme::install_fonts(&context);
        app.logs_window_open = true;
        let output = context.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw(ui));
        });
        assert!(!output.shapes.is_empty());
        assert!(app.logs_cached_text.contains("startup_record"));
        assert!(app.logs_cached_text.contains("live_record"));
        app.logs_window_open = false;
        let _ = context.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw(ui));
        });
        app.logs_window_open = true;
        let _ = context.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.draw(ui));
        });
        assert_eq!(PersistedState::from_app(&app.state), persisted);
    }
    #[test]
    fn completed_action_decrements_in_flight_and_records_sent_status() {
        let request = ActionRequest {
            target: None,
            resolved: resolved(15, 3),
            trigger: trigger(TriggerKind::Death, "session", 1),
            queued_at: Instant::now(),
        };
        let (sender, receiver) = mpsc::channel();
        let mut state = AppState {
            action_result: receiver,
            action_in_flight: 1,
            ..AppState::default()
        };
        sender
            .send(ActionCompletion {
                request,
                result: ActionCompletionResult::Completed(Ok(())),
            })
            .unwrap();
        state.poll_action();
        assert_eq!(state.action_in_flight, 0);
        assert!(matches!(state.action_status, Some(ActionStatus::Sent(_))));
    }

    #[test]
    fn no_connection_reports_skip_with_or_without_a_selected_toy() {
        let mut state = AppState::default();
        state.queue_trigger_action(trigger(TriggerKind::Death, "session", 1));
        assert!(matches!(
            state.action_status,
            Some(ActionStatus::Skipped { reason, .. }) if reason == "provider is not connected"
        ));

        let mut connected_shape = AppState {
            devices: vec![ProviderTarget::new("toy-1", "hub")],
            selected_device: Some("toy-1".to_owned()),
            ..AppState::default()
        };
        connected_shape.queue_trigger_action(trigger(TriggerKind::Death, "session", 1));
        assert!(matches!(
            connected_shape.action_status,
            Some(ActionStatus::Skipped { reason, .. }) if reason == "provider is not connected"
        ));
    }
}
