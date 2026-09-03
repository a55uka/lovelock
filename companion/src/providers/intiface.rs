use buttplug_client::ButtplugClient;
use buttplug_client::connector::ButtplugRemoteClientConnector;
use buttplug_client::serializer::ButtplugClientJSONSerializer;
use buttplug_transport_websocket_tungstenite::ButtplugWebsocketClientTransport;

use crate::provider::{IntifaceSetup, ProviderError};
use crate::providers::ProviderImpl;
use crate::providers::buttplug::{ButtplugBackend, buttplug_error};

pub fn connect(setup: &IntifaceSetup) -> Result<Box<dyn ProviderImpl>, ProviderError> {
    if !setup.present() {
        return Err(ProviderError::InvalidSetup);
    }
    let rt = crate::providers::runtime()?;
    let connector = ButtplugRemoteClientConnector::<
        ButtplugWebsocketClientTransport,
        ButtplugClientJSONSerializer,
    >::new(ButtplugWebsocketClientTransport::new_insecure_connector(
        setup.websocket_url.trim(),
    ));
    let client = ButtplugClient::new("Lovelock Companion");
    rt.block_on(client.connect(connector))
        .map_err(buttplug_error)?;
    // The external server scans on its own; we just register with it.
    Ok(Box::new(ButtplugBackend::new(rt, client)))
}
