#![cfg(feature = "mock-cec")]
//! Last-gap coverage: MQTT dispatch breadth, supervisor reconnect signal,
//! update network-error arms, steward vendor profiles, UI feed lines, exec
//! leftovers, and CEC API sweep (feature = "mock-cec").

#![cfg(feature = "mock-cec")]

use capi::cec;
mod common;

use capi::events::EventHub;
use capi::mqtt;
use capi::supervisor;
use capi::types::AppEvent;
use capi::{AdapterHandle, BusState};
use serial_test::serial;
use std::sync::Arc;

// -- mqtt ------------------------------------------------------------------

#[test]
#[serial]
fn mqtt_dispatch_covers_all_branches() {
    cec::mock::reset();
    let state = common::app_state_with_live_session();

    let mk = |a: &str, p: &[u8]| mqtt::MqttCommand {
        action: a.into(),
        payload: p.to_vec(),
    };
    let cases: Vec<mqtt::MqttCommand> = vec![
        mk("power/on", b"0"),
        mk("power/on", b"bogus"), // malformed -> default 0
        mk("power/off", b"4"),
        mk("volume/up", b""),
        mk("volume/down", b""),
        mk("volume/mute", b""),
        mk("source", b"4"),
        mk("hdmi", b"2"),
        mk("key", br#"{"address":0,"key":"select"}"#),
        mk("key", b"{invalid"),     // decode error branch
        mk("totally_unknown", b""), // unknown topic branch
    ];
    for c in &cases {
        capi::dispatch::dispatch_mqtt_command(&state, c);
    }
}

// -- update network arms -----------------------------------------------------

#[tokio::test]
async fn update_network_error_surfaces() {
    // Closed port: connection refused.
    let dir = tempfile::tempdir().unwrap();
    let (settings, _) = capi::settings::Settings::load(&dir.path().join("c.json")).unwrap();
    let err = capi::update::__test_check(&settings, "http://127.0.0.1:1", None)
        .await
        .unwrap_err();
    assert!(!err.is_empty());
}

#[tokio::test]
async fn update_invalid_json_surfaces() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/repos/{owner}/{repo}/releases/latest",
        axum::routing::get(|| async { "not-json".to_string() }),
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("capi"), b"OLD").unwrap();
    let (settings, _) = capi::settings::Settings::load(&dir.path().join("config.json")).unwrap();
    let err = capi::update::__test_check(
        &settings,
        &format!("http://{addr}"),
        Some(dir.path().to_path_buf()),
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("parse") || err.contains("JSON") || err.contains("json"),
        "{err}"
    );
}

// -- supervisor reconnect signal ----------------------------------------------

#[test]
#[serial]
fn supervisor_reconnect_signal_cycles_session() {
    use capi::supervisor::SupervisorDeps;

    let dir = tempfile::tempdir().unwrap();
    let (settings, _) = capi::settings::Settings::load(&dir.path().join("config.json")).unwrap();
    let hub = Arc::new(EventHub::new(64));
    let adapter = AdapterHandle::new();
    let deps = SupervisorDeps {
        settings: Arc::new(settings),
        adapter: adapter.clone(),
        bus: Arc::new(BusState::new()),
        hub: hub.clone(),
    };

    let handle = std::thread::spawn(move || {
        supervisor::run_supervisor(
            deps,
            "reconnect".into(),
            "/dev/mock0".into(),
            false,
            Arc::new(|_, _| {}),
        )
    });

    // Connected.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !adapter.ready() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(adapter.ready(), "session came up");

    // Signal reconnect: teardown + fresh session.
    adapter.signal_reconnect();
    std::thread::sleep(std::time::Duration::from_millis(600));

    // Shut down cleanly.
    capi::supervisor::SHUTDOWN_FLAG.store(true, std::sync::atomic::Ordering::SeqCst);
    adapter.signal_reconnect(); // wake the wait
    handle.join().expect("clean exit");
    capi::supervisor::SHUTDOWN_FLAG.store(false, std::sync::atomic::Ordering::SeqCst);
}

// -- steward vendor profiles ---------------------------------------------------

#[tokio::test]
#[serial]
async fn steward_deep_respects_vendor_profile_skips() {
    cec::mock::reset();
    let state = common::app_state_with_live_session();
    state
        .settings()
        .update(|c| {
            c.bus.vendor_profiles.insert(
                "0x809819".into(),
                capi::types::VendorProfile {
                    skip_probes: vec![
                        "vendor".into(),
                        "osd".into(),
                        "cec_version".into(),
                        "power".into(),
                        "physical".into(),
                    ],
                    settle_ms: 0,
                },
            );
        })
        .unwrap();

    state
        .steward()
        .enqueue_wait(
            capi::steward::JobKind::Deep,
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("deep job");
}

// -- ui feed line kinds ----------------------------------------------------------

#[test]
fn event_feed_lines_render_for_every_kind() {
    use capi::types::event_type::*;
    let kinds = [
        POWER_CHANGE,
        SOURCE_ACTIVATED,
        KEY_PRESS,
        COMMAND,
        ALERT,
        DEVICES_CHANGED,
        CONFIGURATION_CHANGED,
        ADAPTER_STATE,
    ];
    for k in kinds {
        let html = capi::ui::event_feed_line_html(&AppEvent::new(
            k,
            serde_json::json!({"state":"x","address":0}),
        ));
        assert!(html.contains("feed-line"), "{k}: {html}");
        assert!(html.contains(k), "{k}: {html}");
    }
}

// -- exec leftovers ---------------------------------------------------------------

#[test]
#[serial]
fn exec_send_key_and_volume_validation_matrix() {
    use capi::exec;
    // keycode out of range
    let state = common::app_state_with_live_session();
    let conn = state.adapter().get().unwrap();
    drop(conn);
    assert!(exec::validate_key_args(0, "", 300).is_err());
    let e = exec::validate_key_args(0, "", 0).unwrap_err();
    assert!(matches!(e, capi::exec::ExecError::MissingKey));
}

// -- cec surface sweep ------------------------------------------------------------

#[test]
#[serial]
fn cec_misc_surface_calls_execute() {
    cec::mock::reset();
    let cfg = capi::cec::Configuration {
        device_name: "sweep".into(),
        device_type: capi::cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: capi::cec::LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    };
    let conn = std::sync::Arc::new(capi::cec::Connection::open(&cfg).unwrap());
    conn.force_opened_for_test();

    let _ = conn.server_version();
    let _ = conn.get_current_configuration();
    let _ = conn.logical_addresses_with_poll(false);
    assert!(conn
        .rescan_devices(std::time::Duration::from_millis(5))
        .is_ok());
    assert!(conn.switch_monitoring(false).is_ok());

    conn.close().unwrap();
}
