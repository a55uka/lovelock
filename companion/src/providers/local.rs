use buttplug_client::ButtplugClient;
use buttplug_client_in_process::ButtplugInProcessClientConnectorBuilder;
use buttplug_server::{ButtplugServerBuilder, device::ServerDeviceManagerBuilder};
use buttplug_server_device_config::{DeviceConfigurationManager, load_protocol_configs};

use crate::provider::{LocalSetup, ProviderError};
use crate::providers::buttplug::{ButtplugBackend, buttplug_error};
use crate::providers::{ProviderImpl, runtime};

/// The client name reported to the embedded buttplug server.
const CLIENT_NAME: &str = "Lovelock Companion (Embedded)";

/// Loads the device protocol definitions shipped inside the buttplug library
/// (148 protocols in v11, including `lovense`).
///
/// This step is load-bearing and easy to get wrong: a bare
/// `DeviceConfigurationManagerBuilder::default().finish()` builds an *empty*
/// configuration with zero protocols — which is exactly what the upstream
/// `in_process_client` convenience helper does in v11. With an empty config
/// the server still enumerates Bluetooth peripherals (so the logs show toys
/// being "found"), but protocol matching can never succeed and no device ever
/// connects. An external Intiface Central loads the real configuration, which
/// is why the same toy works there but not in the embedded engine.
pub(crate) fn load_device_config() -> Result<DeviceConfigurationManager, ProviderError> {
    let config = load_protocol_configs(&None, &None, false)
        .map_err(buttplug_error)?
        .finish()
        .map_err(buttplug_error)?;
    log::info!(
        target: "companion::providers::local",
        "local_device_config_loaded protocols={} has_lovense={}",
        config.base_communication_specifiers().len(),
        config.base_communication_specifiers().contains_key("lovense"),
    );
    Ok(config)
}

/// Starts an embedded buttplug server and connects the in-process client to
/// it.
///
/// This mirrors the `in_process_client` convenience helper from the buttplug
/// cookbook ("Embedded Servers and Connectors"), but builds the server
/// explicitly so that:
///
/// - the device configuration is actually loaded (see [`load_device_config`]
///   — without it no discovered hardware can ever match a protocol);
/// - the set of device managers cannot silently change under us when the
///   upstream helper gains/loses managers (its own docs recommend the manual
///   builder for anything production);
/// - every fallible step is mapped to a [`ProviderError`] instead of
///   panicking on `unwrap`;
/// - hardware scanning starts before we return, so the first device poll
///   after connect can already see toys (an external Intiface Central is
///   typically already scanning when we connect to it, which is why the same
///   toy shows up there but not here).
pub fn connect(_setup: &LocalSetup) -> Result<Box<dyn ProviderImpl>, ProviderError> {
    let rt = runtime()?;
    let client = rt.block_on(async {
        let device_config = load_device_config()?;
        let mut device_manager = ServerDeviceManagerBuilder::new(device_config);

        // Bluetooth LE — this is the manager that finds a Lovense toy over
        // direct Bluetooth. It needs a powered-on adapter, OS Bluetooth
        // permission, and a toy that is *not* already holding a BLE
        // connection elsewhere (e.g. to the Lovense Remote app).
        device_manager.comm_manager(
            buttplug_server_hwmgr_btleplug::BtlePlugCommunicationManagerBuilder::default(),
        );
        log::info!(
            target: "companion::providers::local",
            "local_device_manager_added manager=btleplug",
        );

        // Lovense toys bridged through the Lovense Remote/Connect app on the
        // LAN (Game Mode). Discovery goes through Lovense's LAN relay, so
        // this path needs internet access plus Game Mode on.
        device_manager.comm_manager(
            buttplug_server_hwmgr_lovense_connect::LovenseConnectServiceCommunicationManagerBuilder::default(
            ),
        );
        log::info!(
            target: "companion::providers::local",
            "local_device_manager_added manager=lovense-connect-service",
        );

        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            device_manager.comm_manager(
                buttplug_server_hwmgr_lovense_dongle::LovenseHIDDongleCommunicationManagerBuilder::default(
                ),
            );
            log::info!(
                target: "companion::providers::local",
                "local_device_manager_added manager=lovense-dongle",
            );
        }

        device_manager.comm_manager(
            buttplug_server_hwmgr_websocket::WebsocketServerDeviceCommunicationManagerBuilder::default(
            )
            .listen_on_all_interfaces(true),
        );
        log::info!(
            target: "companion::providers::local",
            "local_device_manager_added manager=websocket",
        );

        #[cfg(target_os = "windows")]
        {
            device_manager.comm_manager(
                buttplug_server_hwmgr_xinput::XInputDeviceCommunicationManagerBuilder::default(),
            );
            log::info!(
                target: "companion::providers::local",
                "local_device_manager_added manager=xinput",
            );
        }

        let server = ButtplugServerBuilder::new(
            device_manager.finish().map_err(buttplug_error)?,
        )
        .finish()
        .map_err(buttplug_error)?;
        let connector = ButtplugInProcessClientConnectorBuilder::default()
            .server(server)
            .finish();
        let client = ButtplugClient::new(CLIENT_NAME);
        client.connect(connector).await.map_err(buttplug_error)?;
        // Start scanning right away: without this, the first device poll
        // after connect always reports zero devices because no scan has run
        // yet. Scanning stays on until the UI toggles it off or disconnects.
        client.start_scanning().await.map_err(|error| {
            log::warn!(
                target: "companion::providers::local",
                "local_initial_scan_failed error={error}",
            );
            buttplug_error(error)
        })?;
        log::info!(
            target: "companion::providers::local",
            "local_engine_started client={CLIENT_NAME} scanning=true",
        );
        Ok::<_, ProviderError>(client)
    })?;
    Ok(Box::new(ButtplugBackend::new(rt, client)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_device_config_must_include_lovense() {
        // Regression test: an empty device configuration enumerates
        // peripherals but can never match them to a protocol, so no device
        // ever connects. This guards the `load_protocol_configs` step above.
        let config = load_device_config().expect("embedded device config must load");
        assert!(
            !config.base_communication_specifiers().is_empty(),
            "embedded device config must not be empty",
        );
        assert!(
            config.base_communication_specifiers().contains_key("lovense"),
            "embedded device config must include the lovense protocol",
        );
        assert!(
            !config.base_device_definitions().is_empty(),
            "embedded device definitions must not be empty",
        );
    }
}
