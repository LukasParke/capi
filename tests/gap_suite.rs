#![cfg(feature = "mock-cec")]
//! Final-gap sweep: exercises remaining uncovered arms across api, exec,
//! ui, update, and strategy execution (feature = "mock-cec").

#![cfg(feature = "mock-cec")]

use capi::cec;
use std::sync::Arc;
mod common;

use axum::body::Body;
use common::*;
use serial_test::serial;
use tower::ServiceExt;

async fn call(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<(String, String)>, // (data, content-type)
) -> (axum::http::StatusCode, String) {
    let mut b = axum::http::Request::builder().method(method).uri(uri);
    let (data, ct) = body
        .map(|(d, c)| (Some(d), Some(c)))
        .unwrap_or((None, None));
    if let Some(ct) = &ct {
        b = b.header("Content-Type", ct.as_str());
    }
    let req = b.body(Body::from(data.unwrap_or_default())).unwrap();
    let resp = app.oneshot(req).await.unwrap();
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

fn live_router() -> (axum::Router, Arc<capi::cec::Connection>) {
    cec::mock::reset();
    let state = app_state_with_live_session();
    state.bus().set_frame_ring_capacity(256);
    let conn = state.adapter().get().expect("live session attached");
    (capi::server::build_router(state), conn)
}

// -- api arms -----------------------------------------------------------------

#[tokio::test]
#[serial]
async fn api_device_happy_path() {
    let (app, _conn) = live_router();
    let (status, body) = call(app, "GET", "/api/devices/4", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"logical_address\":4"), "{body}");
}

#[tokio::test]
#[serial]
async fn api_power_status_happy() {
    let (app, _conn) = live_router();
    let (status, body) = call(app, "GET", "/api/power/status", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("On") || body.contains("on"), "{body}");
}

#[tokio::test]
#[serial]
async fn api_volume_and_source_happy() {
    let (app, _conn) = live_router();
    for uri in [
        "/api/volume/up",
        "/api/volume/down",
        "/api/volume/mute?address=5",
        "/api/source/active",
    ] {
        let method = if uri.contains("/mute") || uri.starts_with("/api/volume") {
            "POST"
        } else {
            "GET"
        };
        let (status, _body) = call(app.clone(), method, uri, None).await;
        assert!(status.is_success(), "{uri}: {status}");
    }
}

#[tokio::test]
#[serial]
async fn api_raw_command_happy() {
    let (app, _conn) = live_router();
    let (status, body) = call(
        app,
        "POST",
        "/api/command",
        Some((
            r#"{"initiator":4,"destination":0,"opcode":143}"#.to_string(),
            "application/json".to_string(),
        )),
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
#[serial]
async fn api_settings_invalid_json_is_400() {
    let (app, _conn) = live_router();
    let (status, _) = call(
        app,
        "POST",
        "/api/settings/mqtt",
        Some(("not-json".to_string(), "application/json".to_string())),
    )
    .await;
    assert_eq!(status, 400);
}

#[tokio::test]
#[serial]
async fn api_key_invalid_json_is_400() {
    let (app, _conn) = live_router();
    let (status, _) = call(
        app,
        "POST",
        "/api/key",
        Some(("nope".to_string(), "application/json".to_string())),
    )
    .await;
    assert_eq!(status, 400);
}

#[tokio::test]
#[serial]
async fn api_logs_frames_topology_health_shapes() {
    let (app, _conn) = live_router();
    for uri in [
        "/api/logs",
        "/api/bus/frames",
        "/api/topology",
        "/api/health",
    ] {
        let (status, body) = call(app.clone(), "GET", uri, None).await;
        assert_eq!(status, 200, "{uri}");
        assert!(body.contains("\"status\":\"success\""), "{uri}: {body}");
    }
}

// -- exec / strategy arms ------------------------------------------------------

#[test]
#[serial]
fn switch_hdmi_port_success_and_validation() {
    cec::mock::reset();
    let cfg = capi::cec::Configuration {
        device_name: "g".into(),
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

    // Valid port: primary path succeeds via mock.
    assert!(capi::exec::switch_to_hdmi_port(&conn, 3).is_ok());
    // Invalid port: validation error before ffi.
    assert!(capi::exec::switch_to_hdmi_port(&conn, 0).is_err());
    assert!(capi::exec::switch_to_hdmi_port(&conn, 16).is_err());
    // Device-switch with invalid LA errors.
    assert!(capi::exec::switch_to_device(&conn, capi::cec::LogicalAddress(15)).is_err());

    conn.close().unwrap();
}

#[test]
#[serial]
fn run_action_monitor_only_refusal() {
    cec::mock::reset();
    let cfg = capi::cec::Configuration {
        monitor_only: true,
        ..monitor_cfg()
    };
    fn monitor_cfg() -> capi::cec::Configuration {
        capi::cec::Configuration {
            device_name: "m".into(),
            device_type: capi::cec::DeviceType::RECORDING,
            physical_address: 0xFFFF,
            base_device: capi::cec::LogicalAddress::TV,
            hdmi_port: 1,
            monitor_only: false,
            activate_source: false,
            wake_devices: vec![],
            power_off_devices: vec![],
        }
    }
    let conn = std::sync::Arc::new(capi::cec::Connection::open(&cfg).unwrap());
    conn.force_opened_for_test();

    let registry = capi::strategies::Registry::new();
    let bus = capi::BusState::new();
    // Registry refuses monitor-only sessions with an explicit Skipped result.
    let results = registry.run(
        &conn,
        &bus,
        capi::strategies::Action::VolumeUp,
        &capi::strategies::RunOptions::default(),
        std::time::Instant::now() + std::time::Duration::from_secs(5),
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, capi::strategies::StratStatus::Skipped);
    assert!(results[0].error.contains("monitor-only"));

    conn.close().unwrap();
}

#[test]
#[serial]
fn registry_run_deadline_exceeded_branch() {
    use capi::strategies::{Action, Registry, RunOptions, Step, StepKind, Strategy};
    cec::mock::reset();
    let cfg = capi::cec::Configuration {
        device_name: "d".into(),
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

    let registry = Registry::new();
    let bus = capi::BusState::new();

    // Deadline already in the past: Run returns immediately with no results.
    let past = std::time::Instant::now() - std::time::Duration::from_secs(10);
    let results = registry.run(&conn, &bus, Action::Power, &RunOptions::default(), past);
    assert!(results.is_empty(), "deadline-first yields no results");

    // Wait-step deadline branch: step sleeps past deadline -> Error.
    let chain = vec![Strategy {
        name: "waiter".into(),
        steps: vec![Step {
            kind: StepKind::Wait,
            target: capi::cec::LogicalAddress::UNKNOWN,
            key: capi::cec::Keycode(0),
            wait: false,
            hold_ms: 0,
            opcode: capi::cec::Opcode(0),
            params: vec![],
            delay_ms: 5000,
        }],
        observe_ms: 0,
    }];
    registry.set_vendor_override("dl", Action::VolumeUp, chain);
    let soon = std::time::Instant::now() + std::time::Duration::from_millis(100);
    let results2 = registry.run(
        &conn,
        &bus,
        Action::VolumeUp,
        &RunOptions {
            vendor: "dl".into(),
            ..Default::default()
        },
        soon,
    );
    assert!(!results2.is_empty());
    assert!(matches!(
        results2[0].status,
        capi::strategies::StratStatus::Error
    ));

    conn.close().unwrap();
}
