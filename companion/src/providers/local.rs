use buttplug_client_in_process::in_process_client;

use crate::provider::{LocalSetup, ProviderError};
use crate::providers::buttplug::ButtplugBackend;
use crate::providers::{ProviderImpl, runtime};

/// The client name reported to the embedded buttplug server.
const CLIENT_NAME: &str = "Lovelock Companion (Embedded)";

/// Starts an embedded buttplug server (with the device managers shipped for the
/// current platform) and connects the in-process client to it. Scanning is
/// intentionally left off until the user toggles it via the UI.
pub fn connect(_setup: &LocalSetup) -> Result<Box<dyn ProviderImpl>, ProviderError> {
    let rt = runtime()?;
    let client = rt.block_on(in_process_client(CLIENT_NAME));
    Ok(Box::new(ButtplugBackend::new(rt, client)))
}
