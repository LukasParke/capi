//! Coverage ratchet push: settings empty/edges, api happy+error arms,
//! cec from_name/display sweeps, exec matrix, steward queue-full at HTTP
//! level, percent-decode unit (feature = "mock-cec").

#![cfg(feature = "mock-cec")]

mod common;

use axum::body::Body;
use capi::cec::{self, LogicalAddress};
use common::*;
use serial_test::serial;
use std::sync::Arc;
use tower::ServiceExt;

async fn call(
    app: axum::Router,
    method: &str,
    uri: &str,
    json: Option<String>,
) -> (axum::http::StatusCode, String) {
    let mut b = axum::http::Request::builder().method(method).uri(uri);
    if json.is_some() {
        b = b.header("Content-Type", "application/json");
    }
    let data = json.unwrap_or_default();
    let resp = app
        .oneshot(b.body(Body::from(data)).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let text = String::from_utf8_lossy(
        &http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes(),
    )
    .to_string();
    (status, text)
}

fn live() -> (axum::Router, Arc<cec::Connection>) {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let conn = state.adapter().get().expect("live");
    (capi::server::build_router(state), conn)
}

// -- api arms ------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn api_devices_wait_parse_error_is_400_with_adapter_up() {
    // With adapter up we reach parse; invalid wait -> 400.
    let (app, _conn) = live();
    let (status, _) = call(app, "GET", "/api/devices?wait=notanumber", None).await;
    assert_eq!(status, 400);
}

#[tokio::test]
#[serial]
async fn api_power_status_addr15_is_400_live() {
    let (app, _conn) = live();
    let (status, _) = call(app, "GET", "/api/power/status/15", None).await;
    assert_eq!(status, 400);
}

#[tokio::test]
#[serial]
async fn api_keycode_out_of_range_400() {
    let (app, _conn) = live();
    let (status, _) = call(
        app,
        "POST",
        "/api/key",
        Some(r#"{"address":0,"keycode":300}"#.to_string()),
    )
    .await;
    assert_eq!(status, 400);
}

// -- cec from_name + opcode display sweeps ----------------------------------------

#[test]
fn keycode_from_name_unknown_and_all_table_entries_resolve() {
    assert!(cec::keycode_from_name("definitely_not_a_key").is_none());
    // Every named key in the table resolves back.
    for (name, code) in cec::keycode_names() {
        assert_eq!(cec::keycode_from_name(&name).unwrap().0, code);
        assert!(!capi::cec::opcode_name(cec::Opcode(code)).is_empty());
    }
}

#[test]
fn logical_address_display_full_range() {
    for b in 0u8..=255 {
        let s = capi::cec::logical_address_name(b);
        assert!(!s.is_empty(), "{b}");
    }
}

#[test]
fn device_type_display_reserved_arm() {
    assert_eq!(capi::cec::DeviceType(9).to_string(), "Reserved");
}

// -- exec validation arms ------------------------------------------------------------

#[test]
fn exec_switch_invalid_ports_and_la() {
    cec::mock::reset();
    let cfg = capi::cec::Configuration {
        device_name: "x".into(),
        device_type: capi::cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    };
    let conn = std::sync::Arc::new(capi::cec::Connection::open(&cfg).unwrap());
    conn.force_opened_for_test();
    assert!(capi::exec::switch_to_hdmi_port(&conn, 16).is_err());
    assert!(capi::exec::switch_to_hdmi_port(&conn, 0).is_err());
    conn.close().unwrap();
}

// -- settings overlay prefix default when file has none --------------------------------

#[test]
fn overlay_keeps_explicit_prefix_from_file() {
    let dir = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
    let p = dir.path().join("config.json");
    std::fs::write(&p, r#"{"mqtt":{"prefix":"fileprefix"}}"#).unwrap();
    let (s, _) = capi::settings::Settings::load(&p).unwrap();
    s.apply_overrides(&capi::settings::CliOverrides {
        mqtt_broker: None,
        mqtt_user: None,
        mqtt_pass: None,
        mqtt_prefix_explicit: false,
        mqtt_prefix: String::new(),
        token: None,
    });
    assert_eq!(s.get().mqtt.prefix, "fileprefix");
}

// -- bridge null/closed arms ------------------------------------------------------------

#[test]
#[serial]
fn bridge_null_and_closed_arms() {
    cec::mock::reset();

    // 1. NULL cb_param (no session ever registered): every emitter no-ops.
    capi::cec::mock::emit_log_null();
    capi::cec::mock::emit_command_detached(0, 4, 0x8F);
    capi::cec::mock::emit_keypress_detached(1, 10);
    capi::cec::mock::emit_alert_detached(1, 0, 0);
    capi::cec::mock::emit_source_activated_detached(4, true);
    capi::cec::mock::emit_config_changed_detached();
    capi::cec::mock::emit_menu_detached(1);

    // 2. Registered-then-closed session: same silent-gate path.
    let cfg = cec::Configuration {
        device_name: "nullarms".into(),
        device_type: cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    };
    let conn = Arc::new(cec::Connection::open(&cfg).unwrap());
    conn.force_opened_for_test();
    conn.close().unwrap(); // deregisters id

    // Re-register is gone; emitters with the stale id are silent.
    // (mock_cb_param still points at old id; session_for returns None.)
    cec::mock::set_fail_next(0);
    capi::cec::mock::emit_log_null();

    // 3. NULL log message through a live session covers String::new() arm.
    let live = Arc::new(cec::Connection::open(&cfg).unwrap());
    live.force_opened_for_test();
    capi::cec::mock::emit_log_null();
    std::thread::sleep(std::time::Duration::from_millis(50));
    live.close().unwrap();

    // Menu handler None path covered by emit_menu_on earlier tests.
}
