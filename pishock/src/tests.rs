use std::net::TcpListener;
use std::thread::{self, JoinHandle};

use mockito::{Matcher, Mock, Server};
use serde_json::{Value, json};

use super::*;

const USERNAME: &str = "api-user";
const API_KEY: &str = "super-secret-key";
const SENDER: &str = "desktop-companion";
const SHARE_CODE: &str = "SHARE-CODE";

fn credentials() -> Credentials {
    Credentials::new(USERNAME, API_KEY)
}

fn auth_mock(server: &mut Server) -> Mock {
    server
        .mock("GET", "/Auth/GetUserIfAPIKeyValid")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("apikey".into(), API_KEY.into()),
            Matcher::UrlEncoded("username".into(), USERNAME.into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"UserID":42,"Ignored":"value"}"#)
        .expect(1)
        .create()
}

fn websocket_client(server: &mut Server) -> (WebSocketClient, Mock) {
    let authentication = auth_mock(server);
    let url = server.url();
    let client = WebSocketClient::connect_to(
        credentials(),
        SENDER.into(),
        websocket::WebSocketUrls {
            auth: url.clone(),
            platform: url.clone(),
            websocket: url,
        },
    )
    .unwrap();
    (client, authentication)
}

fn legacy_client(server: &Server) -> LegacyClient {
    LegacyClient::new_to(credentials(), SENDER.into(), server.url()).unwrap()
}

fn operation_mock(server: &mut Server, body: &str, response: &str) -> Mock {
    server
        .mock("POST", "/api/apioperate/")
        .match_header("content-type", Matcher::Regex("^application/json".into()))
        .match_body(Matcher::JsonString(body.into()))
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_body(response)
        .expect(1)
        .create()
}

fn start_websocket_server(response: impl Into<String>) -> (String, JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let response = response.into();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        let message = socket.read().unwrap();
        let tungstenite::Message::Text(body) = message else {
            panic!("expected text command");
        };
        let payload = serde_json::from_str(body.as_str()).unwrap();
        socket
            .send(tungstenite::Message::Text(response.into()))
            .unwrap();
        payload
    });
    (format!("ws://{address}/v2"), handle)
}

fn owned_device(shockers: Vec<Shocker>) -> Device {
    Device {
        client_id: 7,
        name: "Hub".into(),
        user_id: 42,
        username: USERNAME.into(),
        shockers,
    }
}

#[test]
fn credentials_debug_redacts_api_key() {
    let rendered = format!("{:?}", credentials());
    assert!(rendered.contains("[REDACTED]"));
    assert!(!rendered.contains(API_KEY));
}
#[test]
fn credentials_trim_boundary_values_and_sender_names() {
    let credentials = Credentials::new(" \tapi-user\n", "\tsuper-secret-key \n");
    assert_eq!(credentials.username, USERNAME);
    assert_eq!(credentials.api_key, API_KEY);
    let rendered = format!("{credentials:?}");
    assert!(!rendered.contains(API_KEY));

    let mut server = Server::new();
    let authentication = auth_mock(&mut server);
    let url = server.url();
    let _client = WebSocketClient::connect_to(
        credentials,
        " \t desktop-companion \n".into(),
        websocket::WebSocketUrls {
            auth: url.clone(),
            platform: url.clone(),
            websocket: url,
        },
    )
    .unwrap();
    authentication.assert();
}

#[test]
fn legacy_client_validates_credentials_when_constructed() {
    assert_eq!(
        LegacyClient::new(Credentials::new("", API_KEY), SENDER).err(),
        Some(Error::EmptyUsername)
    );
    assert_eq!(
        LegacyClient::new(Credentials::new(USERNAME, " "), SENDER).err(),
        Some(Error::EmptyApiKey)
    );
    assert_eq!(
        LegacyClient::new(credentials(), "\t").err(),
        Some(Error::EmptySender)
    );
    assert!(LegacyClient::new(credentials(), SENDER).is_ok());
}
#[test]
fn whitespace_only_share_codes_are_rejected_before_requests() {
    let server = Server::new();
    let client = legacy_client(&server);
    assert_eq!(client.get_shocker_info(" \t\n"), Err(Error::EmptyShareCode));
    assert_eq!(
        client.send_command(" \t\n", Command::Beep { duration: 1 }),
        Err(Error::EmptyShareCode)
    );
}

#[test]
fn connect_and_list_devices_obey_discovery_contracts() {
    let mut server = Server::new();
    let (client, authentication) = websocket_client(&mut server);
    authentication.assert();
    let listing = server
        .mock("GET", "/PiShock/GetUserDevices")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("UserId".into(), "42".into()),
            Matcher::UrlEncoded("Token".into(), API_KEY.into()),
            Matcher::UrlEncoded("api".into(), "true".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"[{"clientId":7,"name":"Hub","userId":42,"username":"api-user","shockers":[{"name":"Collar","shockerId":9,"isPaused":false}]}]"#)
        .expect(1)
        .create();

    let devices = client.list_devices().unwrap();

    listing.assert();
    assert_eq!(devices[0].client_id, 7);
    assert_eq!(devices[0].shockers[0].shocker_id, 9);
}

#[test]
fn get_device_lists_and_selects_by_client_id() {
    let mut server = Server::new();
    let (client, _authentication) = websocket_client(&mut server);
    let listing = server
        .mock("GET", "/PiShock/GetUserDevices")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("UserId".into(), "42".into()),
            Matcher::UrlEncoded("Token".into(), API_KEY.into()),
            Matcher::UrlEncoded("api".into(), "true".into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"[{"clientId":1,"name":"First","userId":42,"username":"api-user","shockers":[]},{"clientId":2,"name":"Second","userId":42,"username":"api-user","shockers":[]}]"#)
        .expect(2)
        .create();

    assert_eq!(client.get_device(2).unwrap().unwrap().name, "Second");
    assert_eq!(client.get_device(99), Ok(None));
    listing.assert();
}

#[test]
fn get_shocker_info_uses_pascal_request_and_parses_camel_response() {
    let mut server = Server::new();
    let client = legacy_client(&server);
    let info_request = server

        .mock("POST", "/api/GetShockerInfo")
        .match_header("content-type", Matcher::Regex("^application/json".into()))
        .match_body(Matcher::JsonString(format!(r#"{{"Username":"{USERNAME}","Code":"{SHARE_CODE}","Apikey":"{API_KEY}"}}"#)))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"Collar","clientId":7,"id":9,"paused":false,"maxIntensity":60,"maxDuration":12}"#)
        .expect(1)
        .create();

    let info = client.get_shocker_info(SHARE_CODE).unwrap();

    info_request.assert();
    assert_eq!(
        info,
        ShockerInfo {
            client_id: 7,
            id: 9,
            name: "Collar".into(),
            paused: false,
            max_intensity: 60,
            max_duration: 12,
            online: None,
        }
    );
}
#[test]
fn legacy_requests_emit_trimmed_credentials_sender_and_share_code() {
    let mut server = Server::new();
    let client = LegacyClient::new_to(
        Credentials::new(" \tapi-user\n", "\tsuper-secret-key \n"),
        " \t desktop-companion \n".into(),
        server.url(),
    )
    .unwrap();
    let request = operation_mock(
        &mut server,
        &format!(
            r#"{{"Username":"{USERNAME}","Name":"{SENDER}","Code":"{SHARE_CODE}","Intensity":25,"Duration":3,"Apikey":"{API_KEY}","Op":0}}"#
        ),
        legacy::OPERATION_SUCCEEDED,
    );

    client.shock("\t SHARE-CODE \n", 25, 3).unwrap();
    request.assert();
}

#[test]
fn get_shocker_info_maps_not_found_and_authorization_statuses() {
    for (status, expected) in [
        (404, Error::ShareCodeNotFound),
        (401, Error::NotAuthorized),
        (403, Error::NotAuthorized),
    ] {
        let mut server = Server::new();
        let client = legacy_client(&server);
        let response = server
            .mock("POST", "/api/GetShockerInfo")
            .with_status(status)
            .expect(1)
            .create();

        assert_eq!(client.get_shocker_info(SHARE_CODE), Err(expected));
        response.assert();
    }
}

#[test]
fn convenience_commands_emit_exact_operation_payloads() {
    let mut server = Server::new();
    let client = legacy_client(&server);

    let shock = operation_mock(
        &mut server,
        &format!(
            r#"{{"Username":"{USERNAME}","Name":"{SENDER}","Code":"{SHARE_CODE}","Intensity":25,"Duration":3,"Apikey":"{API_KEY}","Op":0}}"#
        ),
        "Operation Succeeded.\n",
    );
    client.shock(SHARE_CODE, 25, 3).unwrap();
    shock.assert();
    drop(shock);

    let vibrate = operation_mock(
        &mut server,
        &format!(
            r#"{{"Username":"{USERNAME}","Name":"{SENDER}","Code":"{SHARE_CODE}","Intensity":80,"Duration":4,"Apikey":"{API_KEY}","Op":1}}"#
        ),
        " Operation Succeeded. ",
    );
    client.vibrate(SHARE_CODE, 80, 4).unwrap();
    vibrate.assert();
    drop(vibrate);

    let beep = operation_mock(
        &mut server,
        &format!(
            r#"{{"Username":"{USERNAME}","Name":"{SENDER}","Code":"{SHARE_CODE}","Duration":2,"Apikey":"{API_KEY}","Op":2}}"#
        ),
        "Operation Succeeded.",
    );
    client.beep(SHARE_CODE, 2).unwrap();
    beep.assert();
}

#[test]
fn beep_device_publishes_to_every_unpaused_shocker() {
    let (websocket_url, websocket_server) = start_websocket_server(
        r#"{"ErrorCode":null,"IsError":false,"Message":"Publish successful.","OriginalCommand":"PUBLISH"}"#,
    );
    let mut server = Server::new();
    let (mut client, _authentication) = websocket_client(&mut server);
    client.urls.websocket = websocket_url;
    let device = owned_device(vec![
        Shocker {
            name: "One".into(),
            shocker_id: 9,
            is_paused: false,
        },
        Shocker {
            name: "Paused".into(),
            shocker_id: 10,
            is_paused: true,
        },
        Shocker {
            name: "Two".into(),
            shocker_id: 11,
            is_paused: false,
        },
    ]);

    client.beep_device(&device, 2).unwrap();

    assert_eq!(
        websocket_server.join().unwrap(),
        json!({
            "Operation": "PUBLISH",
            "PublishCommands": [
                {
                    "Target": "c7-ops",
                    "Body": {
                        "id": 9,
                        "m": "b",
                        "i": 0,
                        "d": 2000,
                        "r": true,
                        "l": {
                            "u": 42,
                            "ty": "api",
                            "w": false,
                            "h": false,
                            "o": SENDER
                        }
                    }
                },
                {
                    "Target": "c7-ops",
                    "Body": {
                        "id": 11,
                        "m": "b",
                        "i": 0,
                        "d": 2000,
                        "r": true,
                        "l": {
                            "u": 42,
                            "ty": "api",
                            "w": false,
                            "h": false,
                            "o": SENDER
                        }
                    }
                }
            ]
        })
    );
}

#[test]
fn shock_device_publishes_exact_payload_to_every_unpaused_shocker() {
    let (websocket_url, websocket_server) = start_websocket_server(
        r#"{"ErrorCode":null,"IsError":false,"Message":"Publish successful.","OriginalCommand":"PUBLISH"}"#,
    );
    let mut server = Server::new();
    let (mut client, _authentication) = websocket_client(&mut server);
    client.urls.websocket = websocket_url;
    let device = owned_device(vec![
        Shocker {
            name: "One".into(),
            shocker_id: 9,
            is_paused: false,
        },
        Shocker {
            name: "Paused".into(),
            shocker_id: 10,
            is_paused: true,
        },
        Shocker {
            name: "Two".into(),
            shocker_id: 11,
            is_paused: false,
        },
    ]);

    client.shock_device(&device, 37, 1_234).unwrap();

    assert_eq!(
        websocket_server.join().unwrap(),
        json!({
            "Operation": "PUBLISH",
            "PublishCommands": [
                {
                    "Target": "c7-ops",
                    "Body": {
                        "id": 9,
                        "m": "s",
                        "i": 37,
                        "d": 1234,
                        "r": true,
                        "l": {
                            "u": 42,
                            "ty": "api",
                            "w": false,
                            "h": false,
                            "o": SENDER
                        }
                    }
                },
                {
                    "Target": "c7-ops",
                    "Body": {
                        "id": 11,
                        "m": "s",
                        "i": 37,
                        "d": 1234,
                        "r": true,
                        "l": {
                            "u": 42,
                            "ty": "api",
                            "w": false,
                            "h": false,
                            "o": SENDER
                        }
                    }
                }
            ]
        })
    );
}

#[test]
fn shock_device_validates_before_networking_and_requires_available_shockers() {
    let mut server = Server::new();
    let (mut client, _authentication) = websocket_client(&mut server);
    client.urls.websocket = "not-a-websocket-url".into();
    let active = owned_device(vec![Shocker {
        name: "Active".into(),
        shocker_id: 9,
        is_paused: false,
    }]);
    let paused = owned_device(vec![Shocker {
        name: "Paused".into(),
        shocker_id: 10,
        is_paused: true,
    }]);

    assert_eq!(
        client.shock_device(&active, 0, 1_000),
        Err(Error::InvalidIntensity)
    );
    assert_eq!(
        client.shock_device(&active, 101, 1_000),
        Err(Error::InvalidIntensity)
    );
    assert_eq!(
        client.shock_device(&active, 50, 299),
        Err(Error::InvalidWebSocketDuration)
    );
    assert_eq!(
        client.shock_device(&active, 50, 65_536),
        Err(Error::InvalidWebSocketDuration)
    );
    assert_eq!(
        client.shock_device(&paused, 50, 1_000),
        Err(Error::NoAvailableShockers)
    );
}

#[test]
fn shock_device_redacts_api_keys_from_rejections_and_attempts_once() {
    let response = format!(
        r#"{{"ErrorCode":"Rejected","IsError":true,"Message":"command rejected for {API_KEY}","OriginalCommand":"PUBLISH"}}"#
    );
    let (websocket_url, websocket_server) = start_websocket_server(response);
    let mut server = Server::new();
    let (mut client, _authentication) = websocket_client(&mut server);
    client.urls.websocket = websocket_url;
    let device = owned_device(vec![Shocker {
        name: "Active".into(),
        shocker_id: 9,
        is_paused: false,
    }]);

    let error = client.shock_device(&device, 1, 300).unwrap_err();

    websocket_server.join().unwrap();
    assert!(matches!(error, Error::WebSocketRejected { .. }));
    assert!(!format!("{error:?} {error}").contains(API_KEY));
}

#[test]
fn beep_device_validates_duration_and_available_shockers() {
    let mut server = Server::new();
    let (client, _authentication) = websocket_client(&mut server);
    let active = owned_device(vec![Shocker {
        name: "Active".into(),
        shocker_id: 9,
        is_paused: false,
    }]);
    let paused = owned_device(vec![Shocker {
        name: "Paused".into(),
        shocker_id: 10,
        is_paused: true,
    }]);

    assert_eq!(client.beep_device(&active, 0), Err(Error::InvalidDuration));
    assert_eq!(
        client.beep_device(&paused, 1),
        Err(Error::NoAvailableShockers)
    );
}

#[test]
fn beep_device_redacts_api_keys_from_websocket_rejections() {
    let response = format!(
        r#"{{"ErrorCode":"Rejected","IsError":true,"Message":"command rejected for {API_KEY}","OriginalCommand":"PUBLISH"}}"#
    );
    let (websocket_url, websocket_server) = start_websocket_server(response);
    let mut server = Server::new();
    let (mut client, _authentication) = websocket_client(&mut server);
    client.urls.websocket = websocket_url;
    let device = owned_device(vec![Shocker {
        name: "Active".into(),
        shocker_id: 9,
        is_paused: false,
    }]);

    let error = client.beep_device(&device, 1).unwrap_err();

    websocket_server.join().unwrap();
    assert!(matches!(error, Error::WebSocketRejected { .. }));
    assert!(!format!("{error:?} {error}").contains(API_KEY));
}

#[test]
fn legacy_command_path_sends_the_selected_command() {
    let mut server = Server::new();
    let client = legacy_client(&server);
    let request = operation_mock(
        &mut server,
        &format!(
            r#"{{"Username":"{USERNAME}","Name":"{SENDER}","Code":"{SHARE_CODE}","Intensity":1,"Duration":15,"Apikey":"{API_KEY}","Op":0}}"#
        ),
        legacy::OPERATION_SUCCEEDED,
    );

    client
        .send_command(
            SHARE_CODE,
            Command::Shock {
                intensity: 1,
                duration: 15,
            },
        )
        .unwrap();
    request.assert();
}

#[test]
fn invalid_values_are_rejected_without_networking() {
    let mut server = Server::new();
    let client = legacy_client(&server);
    let no_operation = server.mock("POST", "/api/apioperate/").expect(0).create();
    let no_info = server
        .mock("POST", "/api/GetShockerInfo")
        .expect(0)
        .create();

    assert_eq!(client.shock(" ", 1, 1), Err(Error::EmptyShareCode));
    assert_eq!(client.shock(SHARE_CODE, 0, 1), Err(Error::InvalidIntensity));
    assert_eq!(
        client.vibrate(SHARE_CODE, 101, 1),
        Err(Error::InvalidIntensity)
    );
    assert_eq!(client.beep(SHARE_CODE, 0), Err(Error::InvalidDuration));
    assert_eq!(client.beep(SHARE_CODE, 16), Err(Error::InvalidDuration));
    assert_eq!(client.get_shocker_info("\t"), Err(Error::EmptyShareCode));
    no_operation.assert();
    no_info.assert();
}

#[test]
fn credentials_are_validated_before_authentication_networking() {
    let cases = [
        (Credentials::new("", API_KEY), SENDER, Error::EmptyUsername),
        (Credentials::new(USERNAME, " "), SENDER, Error::EmptyApiKey),
        (
            Credentials::new(USERNAME, API_KEY),
            "\t",
            Error::EmptySender,
        ),
    ];

    for (credentials, sender, expected) in cases {
        let mut server = Server::new();
        let no_auth = server
            .mock("GET", "/Auth/GetUserIfAPIKeyValid")
            .expect(0)
            .create();
        let url = server.url();
        let result = WebSocketClient::connect_to(
            credentials,
            sender.into(),
            websocket::WebSocketUrls {
                auth: url.clone(),
                platform: url.clone(),
                websocket: url,
            },
        );
        assert_eq!(result.err(), Some(expected));
        no_auth.assert();
    }
}

#[test]
fn documented_operation_rejections_map_to_typed_errors() {
    let cases = [
        ("This code doesn’t exist.", Error::ShareCodeNotFound),
        ("Not Authorized.", Error::NotAuthorized),
        (
            "Shocker is Paused, unable to send command.",
            Error::ShockerPaused,
        ),
        ("Device currently not connected.", Error::DeviceOffline),
        (
            "This share code has already been used by somebody else.",
            Error::ShareCodeInUse,
        ),
        (
            "Unknown Op, use 0 for shock, 1 for vibrate and 2 for beep.",
            Error::InvalidOperation,
        ),
    ];

    for (body, expected) in cases {
        let mut server = Server::new();
        let client = legacy_client(&server);
        let rejection = operation_mock(
            &mut server,
            &format!(
                r#"{{"Username":"{USERNAME}","Name":"{SENDER}","Code":"{SHARE_CODE}","Duration":1,"Apikey":"{API_KEY}","Op":2}}"#
            ),
            body,
        );
        assert_eq!(client.beep(SHARE_CODE, 1), Err(expected));
        rejection.assert();
    }
}

#[test]
fn bounded_live_and_unknown_rejections_are_preserved() {
    assert_eq!(
        legacy::parse_operation_response("Intensity must be between 0 and 50", API_KEY),
        Err(Error::IntensityRejected {
            message: "Intensity must be between 0 and 50".into()
        })
    );
    assert_eq!(
        legacy::parse_operation_response("Duration must be between 1 and 8", API_KEY),
        Err(Error::DurationRejected {
            message: "Duration must be between 1 and 8".into()
        })
    );
    assert_eq!(
        legacy::parse_operation_response("Unexpected policy rejection", API_KEY),
        Err(Error::OperationRejected {
            message: "Unexpected policy rejection".into()
        })
    );
    assert_eq!(
        legacy::parse_operation_response("Shock not allowed.", API_KEY),
        Err(Error::OperationNotAllowed)
    );
    assert_eq!(
        legacy::parse_operation_response("Device in Use.", API_KEY),
        Err(Error::ShareCodeInUse)
    );
    assert_eq!(
        legacy::parse_operation_response(&format!("Rejected key {API_KEY}"), API_KEY),
        Err(Error::OperationRejected {
            message: "Rejected key [REDACTED]".into()
        })
    );
}

#[test]
fn a_rejected_command_is_sent_exactly_once() {
    let mut server = Server::new();
    let client = legacy_client(&server);
    let rejection = operation_mock(
        &mut server,
        &format!(
            r#"{{"Username":"{USERNAME}","Name":"{SENDER}","Code":"{SHARE_CODE}","Duration":1,"Apikey":"{API_KEY}","Op":2}}"#
        ),
        "Device in Use.",
    );

    assert_eq!(client.beep(SHARE_CODE, 1), Err(Error::ShareCodeInUse));
    rejection.assert();
}

#[test]
fn http_and_decode_failures_are_typed_and_redacted() {
    let mut status_server = Server::new();
    let status_auth = status_server
        .mock("GET", "/Auth/GetUserIfAPIKeyValid")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("apikey".into(), API_KEY.into()),
            Matcher::UrlEncoded("username".into(), USERNAME.into()),
        ]))
        .with_status(403)
        .expect(1)
        .create();
    let status_url = status_server.url();
    let error = WebSocketClient::connect_to(
        credentials(),
        SENDER.into(),
        websocket::WebSocketUrls {
            auth: status_url.clone(),
            platform: status_url.clone(),
            websocket: status_url,
        },
    )
    .err()
    .unwrap();
    assert_eq!(error, Error::AuthenticationRejected);
    assert!(!format!("{error:?} {error}").contains(API_KEY));
    status_auth.assert();

    let mut decode_server = Server::new();
    let malformed = decode_server
        .mock("GET", "/Auth/GetUserIfAPIKeyValid")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("apikey".into(), API_KEY.into()),
            Matcher::UrlEncoded("username".into(), USERNAME.into()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body("not-json")
        .expect(1)
        .create();
    let decode_url = decode_server.url();
    let error = WebSocketClient::connect_to(
        credentials(),
        SENDER.into(),
        websocket::WebSocketUrls {
            auth: decode_url.clone(),
            platform: decode_url.clone(),
            websocket: decode_url,
        },
    )
    .err()
    .unwrap();
    assert_eq!(
        error,
        Error::Decode {
            operation: "authentication"
        }
    );
    assert!(!format!("{error:?} {error}").contains(API_KEY));
    malformed.assert();
}
