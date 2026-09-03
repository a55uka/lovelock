use crate::action::ResolvedVibrateAction;
use crate::provider::{LovenseSetup, ProviderError, ProviderTarget};
use crate::providers::{ProviderImpl, TEST_VIBRATE_DURATION_SECS, TEST_VIBRATE_STRENGTH};
use lovense::{Connection, LovenseClient};

pub fn connect(setup: &LovenseSetup) -> Result<Box<dyn ProviderImpl>, ProviderError> {
    if !setup.present() {
        return Err(ProviderError::InvalidSetup);
    }
    let connection = Connection::new(setup.domain.trim(), setup.http_port);
    let client = LovenseClient::connect(connection)?;
    Ok(Box::new(LovenseBackend(client)))
}

/// The synchronous Lovense backend. Talks to the Lovense Remote/Connect app on
/// the same LAN over its local Standard API.
struct LovenseBackend(LovenseClient);

impl ProviderImpl for LovenseBackend {
    fn list_targets(&self) -> Result<Vec<ProviderTarget>, ProviderError> {
        Ok(self
            .0
            .list_toys()?
            .into_iter()
            .map(ProviderTarget::from_lovense)
            .collect())
    }

    fn test_action(&self, target: Option<&ProviderTarget>) -> Result<(), ProviderError> {
        let toy = target.and_then(ProviderTarget::toy);
        self.0
            .vibrate(toy, TEST_VIBRATE_STRENGTH, TEST_VIBRATE_DURATION_SECS)?;
        Ok(())
    }

    fn execute(
        &self,
        target: Option<&ProviderTarget>,
        action: ResolvedVibrateAction,
    ) -> Result<(), ProviderError> {
        let toy = target.and_then(ProviderTarget::toy);
        self.0.vibrate(toy, action.strength, action.duration_secs)?;
        Ok(())
    }

    fn disconnect(self: Box<Self>) -> Result<(), ProviderError> {
        // Lovense is stateless HTTP; there is no session to tear down.
        Ok(())
    }
}
