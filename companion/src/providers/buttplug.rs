use std::sync::Arc;

use buttplug_client::{
    ButtplugClient, ButtplugClientDevice,
    device::{ClientDeviceCommandValue, ClientDeviceOutputCommand},
};
use buttplug_core::message::OutputType;

use crate::action::ResolvedVibrateAction;
use crate::provider::{ProviderError, ProviderTarget};
use crate::providers::{ProviderImpl, TEST_VIBRATE_DURATION_SECS, TEST_VIBRATE_STRENGTH};

/// Maps a 0–20 companion strength to a buttplug vibration percentage (0.0–1.0).
pub(crate) fn strength_to_percent(strength: u8) -> f64 {
    (strength as f64 / 20.0).clamp(0.0, 1.0)
}

/// Shared backend for providers that talk to a `ButtplugClient` — both the
/// externally-running Intiface Central and the embedded local engine. Async
/// operations are driven with `block_on` on a shared tokio runtime.
pub(crate) struct ButtplugBackend {
    rt: Arc<tokio::runtime::Runtime>,
    client: ButtplugClient,
}

impl ButtplugBackend {
    pub(crate) fn new(rt: Arc<tokio::runtime::Runtime>, client: ButtplugClient) -> Self {
        Self { rt, client }
    }
}

impl ProviderImpl for ButtplugBackend {
    fn list_targets(&self) -> Result<Vec<ProviderTarget>, ProviderError> {
        let devices = self.client.devices();
        Ok(devices
            .into_values()
            .map(|device: ButtplugClientDevice| {
                ProviderTarget::new_any(device.index().to_string(), device.name())
            })
            .collect())
    }

    fn test_action(&self, target: Option<&ProviderTarget>) -> Result<(), ProviderError> {
        self.execute_any(
            target,
            strength_to_percent(TEST_VIBRATE_STRENGTH),
            TEST_VIBRATE_DURATION_SECS,
        )
    }

    fn execute(
        &self,
        target: Option<&ProviderTarget>,
        action: ResolvedVibrateAction,
    ) -> Result<(), ProviderError> {
        self.execute_any(
            target,
            strength_to_percent(action.strength),
            action.duration_secs,
        )
    }

    fn disconnect(self: Box<Self>) -> Result<(), ProviderError> {
        let _ = self.rt.block_on(self.client.disconnect());
        Ok(())
    }

    fn start_scanning(&self) -> Result<(), ProviderError> {
        self.rt
            .block_on(self.client.start_scanning())
            .map_err(buttplug_error)?;
        Ok(())
    }

    fn stop_scanning(&self) -> Result<(), ProviderError> {
        self.rt
            .block_on(self.client.stop_scanning())
            .map_err(buttplug_error)?;
        Ok(())
    }
}

impl ButtplugBackend {
    fn execute_any(
        &self,
        target: Option<&ProviderTarget>,
        percent: f64,
        duration_secs: u32,
    ) -> Result<(), ProviderError> {
        let devices = self.client.devices();
        let targets: Vec<ButtplugClientDevice> = if let Some(target_id) = target {
            devices
                .values()
                .find(|device| device.index().to_string() == target_id.id().as_str())
                .cloned()
                .into_iter()
                .collect()
        } else {
            devices
                .values()
                .filter(|device| device.output_available(OutputType::Vibrate))
                .cloned()
                .collect()
        };
        if target.is_some() && targets.is_empty() {
            return Err(ProviderError::NotConnected);
        }
        set_vibration(self.rt.clone(), &targets, percent)?;
        if duration_secs > 0 {
            std::thread::sleep(std::time::Duration::from_millis(
                u64::from(duration_secs) * 1000,
            ));
            set_vibration(self.rt.clone(), &targets, 0.0)?;
        }
        Ok(())
    }
}

fn set_vibration(
    rt: Arc<tokio::runtime::Runtime>,
    devices: &[ButtplugClientDevice],
    percent: f64,
) -> Result<(), ProviderError> {
    for device in devices {
        rt.block_on(device.run_output(&ClientDeviceOutputCommand::Vibrate(
            ClientDeviceCommandValue::Percent(percent),
        )))
        .map_err(buttplug_error)?;
    }
    Ok(())
}

pub(crate) fn buttplug_error(error: impl std::fmt::Display) -> ProviderError {
    ProviderError::Buttplug(error.to_string())
}
