pub(crate) mod buttplug;
pub mod intiface;
pub mod local;
pub mod lovense;

use std::sync::Arc;

use crate::action::ResolvedVibrateAction;
use crate::provider::{ProviderError, ProviderTarget};

pub const TEST_VIBRATE_STRENGTH: u8 = 8;
pub const TEST_VIBRATE_DURATION_SECS: u32 = 1;

/// Creates the shared tokio runtime used to drive async (buttplug) backends.
pub(crate) fn runtime() -> Result<Arc<tokio::runtime::Runtime>, ProviderError> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map(Arc::new)
        .map_err(|error| {
            ProviderError::Buttplug(format!("Could not start the embedded runtime: {error}"))
        })
}

/// Connects the selected provider and returns a live `ConnectedProvider`.
pub fn connect(
    settings: &crate::provider::ProviderSettings,
) -> Result<ConnectedProvider, ProviderError> {
    let kind = settings.kind();
    let inner: Box<dyn ProviderImpl> = match settings {
        crate::provider::ProviderSettings::Lovense(setup) => lovense::connect(setup)?,
        crate::provider::ProviderSettings::Local(setup) => local::connect(setup)?,
        crate::provider::ProviderSettings::Intiface(setup) => intiface::connect(setup)?,
    };
    Ok(ConnectedProvider::new(kind, inner))
}

/// Which toy backend the companion should talk to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ProviderKind {
    /// The Lovense Remote/Connect app on the same LAN (local Standard API).
    Lovense,
    /// An embedded buttplug server running inside the companion.
    Local,
    /// An externally running Intiface Central (or any buttplug WebSocket server).
    Intiface,
}

impl ProviderKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lovense => "Lovense",
            Self::Local => "Local (Embedded Intiface)",
            Self::Intiface => "Intiface Central",
        }
    }
    pub fn log_label(self) -> &'static str {
        match self {
            Self::Lovense => "lovense",
            Self::Local => "local",
            Self::Intiface => "intiface",
        }
    }
}

/// A live connection to a toy provider, dispatched across the three
/// backends. All methods are synchronous (they run on the action worker / UI
/// poll threads); async backends block on a shared tokio runtime internally.
pub struct ConnectedProvider {
    kind: ProviderKind,
    inner: Box<dyn ProviderImpl>,
}

impl ConnectedProvider {
    pub fn new(kind: ProviderKind, inner: Box<dyn ProviderImpl>) -> Self {
        Self { kind, inner }
    }
    pub fn kind(&self) -> ProviderKind {
        self.kind
    }
    pub fn list_targets(&self) -> Result<Vec<ProviderTarget>, ProviderError> {
        self.inner.list_targets()
    }
    pub fn test_action(&self, target: Option<&ProviderTarget>) -> Result<(), ProviderError> {
        self.inner.test_action(target)
    }
    pub fn execute(
        &self,
        target: Option<&ProviderTarget>,
        action: ResolvedVibrateAction,
    ) -> Result<(), ProviderError> {
        self.inner.execute(target, action)
    }
    pub fn disconnect(self) -> Result<(), ProviderError> {
        self.inner.disconnect()
    }
    pub fn start_scanning(&self) -> Result<(), ProviderError> {
        self.inner.start_scanning()
    }
    pub fn stop_scanning(&self) -> Result<(), ProviderError> {
        self.inner.stop_scanning()
    }
}

/// The shared, provider-agnostic interface each backend implements.
pub trait ProviderImpl: Send + Sync {
    fn list_targets(&self) -> Result<Vec<ProviderTarget>, ProviderError>;
    fn test_action(&self, target: Option<&ProviderTarget>) -> Result<(), ProviderError>;
    fn execute(
        &self,
        target: Option<&ProviderTarget>,
        action: ResolvedVibrateAction,
    ) -> Result<(), ProviderError>;
    fn disconnect(self: Box<Self>) -> Result<(), ProviderError>;

    /// Toggle hardware scanning on/off. Defaults to no-ops for providers that
    /// don't maintain a scan state (Lovense / external Intiface scan on connect).
    fn start_scanning(&self) -> Result<(), ProviderError> {
        Ok(())
    }
    fn stop_scanning(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}
