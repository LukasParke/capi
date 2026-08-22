#![cfg(feature = "mock-cec")]
//! Live-mock router suite: dev_api + command endpoints against a
//! transmit-capable mock session (feature = "mock-cec").

#![cfg(feature = "mock-cec")]

use capi::cec;
use capi::cec::LogicalAddress;
mod common;

use axum::body::Body;
use common::*;
use serial_test::serial;
use tower::ServiceExt;

async fn post_json(
    app: axum::Router,
    uri: &str,
    json: &str,
) -> (axum::http::StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .body(Body::from(json.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, envelope(&body))
}

async fn get_json(app: &axum::Router, uri: &str) -> serde_json::Value {
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    envelope(&body)
}

#[tokio::test]
#[serial]
async fn probe_runs_all_steps_with_replies() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state.clone());

    let (status, v) = post_json(
        app.clone(),
        "/api/dev/probe",
        r#"{"address":0,"kind":"all","observe_ms":80}"#,
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let steps = v["data"]["steps"].as_array().unwrap();
    assert!(!steps.is_empty());
    // Mock acks every transmit: no step errors.
    for s in steps {
        assert_eq!(s["result"], "ok", "{s}");
    }
}

#[tokio::test]
#[serial]
async fn run_strategies_returns_ok_classification() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state.clone());

    let (status, v) = post_json(
        app.clone(),
        "/api/dev/run_strategies",
        r#"{"action":"volume_up","target":-1,"observe_ms":100}"#,
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let results = v["data"]["results"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["status"], "ok", "{results:?}");
}

#[tokio::test]
#[serial]
async fn send_opcode_roundtrip() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state.clone());

    let (status, v) = post_json(
        app,
        "/api/dev/send_opcode",
        r#"{"destination":0,"opcode":143,"params_hex":"10 02","observe_ms":80}"#,
    )
    .await;
    assert_eq!(status, 200, "{v}");
    // Frame ring captured the outbound frame via the event consumer? The
    // mock acks but does not echo; ring gets frames only from injected
    // callbacks — assert the call succeeded and reported no tx error.
    assert!(v["data"]["transmit_error"].is_null(), "{v}");
}

#[tokio::test]
#[serial]
async fn key_send_ok_via_mock_ack() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state.clone());
    let (status, v) = post_json(app, "/api/key", r#"{"address":0,"key":"select"}"#).await;
    assert_eq!(status, 200, "{v}");
    let msg = v["message"].as_str().unwrap();
    assert!(msg.contains("ok") || msg.contains("tried"), "{msg}");
}

#[tokio::test]
#[serial]
async fn devices_live_lists_mock_devices() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state.clone());
    let v = get_json(&app, "/api/devices").await;
    assert_success(&v);
}

#[test]
#[serial]
fn cec_stats_and_menu_and_injections() {
    cec::mock::reset();
    let cfg = capi::cec::Configuration {
        device_name: "stats".into(),
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

    let mut rx = conn.subscribe_events();

    capi::cec::mock::emit_alert_on(&conn, 2, 1, 7);
    let ev = rx.blocking_recv().unwrap();
    assert!(
        matches!(ev, capi::cec::CecEvent::Alert { alert: 2, .. }),
        "{ev:?}"
    );

    capi::cec::mock::emit_config_changed_on(&conn);
    let ev = rx.blocking_recv().unwrap();
    assert!(
        matches!(ev, capi::cec::CecEvent::ConfigurationChanged(_)),
        "{ev:?}"
    );

    capi::cec::mock::emit_source_activated_on(&conn, 4, true);
    let ev = rx.blocking_recv().unwrap();
    assert!(
        matches!(
            ev,
            capi::cec::CecEvent::SourceActivated {
                address: 4,
                activated: true
            }
        ),
        "{ev:?}"
    );

    // Menu round-trip returns through the synchronous reply path.
    assert_eq!(capi::cec::mock::emit_menu_on(&conn, 1), 1);

    conn.close().unwrap();
}

#[tokio::test]
#[serial]
async fn dev_mode_and_probe_filter_arms() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state);

    // Mode GET.
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/dev/mode")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Probe with each single kind (mock acks; replies may or may not come).
    for kind in ["power", "vendor", "osd", "cec_version", "physical"] {
        let (status, _) = post_json(
            app.clone(),
            "/api/dev/probe",
            &format!(r#"{{"address":0,"kind":"{kind}","observe_ms":40}}"#),
        )
        .await;
        assert_eq!(status, 200, "{kind}");
    }

    // Send key repeat path.
    let (status, _) = post_json(
        app.clone(),
        "/api/dev/send_key",
        r#"{"address":0,"key":"volume_up","repeat":2,"hold_ms":10}"#,
    )
    .await;
    assert_eq!(status, 200);
}

#[tokio::test]
#[serial]
async fn connection_full_success_surface() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let conn = state.adapter().get().unwrap();

    // These succeed against the mock (all acked).
    assert!(conn.audio_mute().is_ok());
    assert!(conn.audio_unmute().is_ok());
    assert!(conn.set_inactive_view().is_ok());
    assert!(conn
        .set_active_source(capi::cec::DeviceType::RECORDING)
        .is_ok());
    assert!(conn
        .set_osd_string(
            LogicalAddress::TV,
            capi::cec::DisplayControl::DEFAULT_TIME,
            "test"
        )
        .is_ok());
    assert!(conn
        .set_configuration(&capi::cec::Configuration {
            device_name: "resuccess".into(),
            ..capi::cec::Configuration {
                device_name: String::new(),
                device_type: capi::cec::DeviceType::RECORDING,
                physical_address: 0xFFFF,
                base_device: LogicalAddress::TV,
                hdmi_port: 1,
                monitor_only: false,
                activate_source: false,
                wake_devices: vec![],
                power_off_devices: vec![],
            }
        })
        .is_ok());
    assert!(conn.server_version().unwrap_or(0) > 0);
    assert!(conn.open_adapter("/dev/mock0").is_ok());

    // get_device_info aggregation.
    let info = conn.get_device_info(capi::cec::LogicalAddress(0)).unwrap();
    let m = info.to_map();
    assert_eq!(m["osd_name"], "MOCKBOX");

    // event_subscribers.
    let _ = conn.event_subscribers();
}
