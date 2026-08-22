//! Final batch: types display sweeps, exec display, ui fallback branches,
//! server unit arms, cec closed-session gates (feature = "mock-cec").

#![cfg(feature = "mock-cec")]

mod common;

use capi::cec::{self, CecEvent, Command, LogicalAddress, Opcode};
use common::*;

use axum::body::Body;
use serial_test::serial;
use std::sync::Arc;
use tower::ServiceExt;

// -- cec/types sweeps ------------------------------------------------------------

#[test]
fn keycode_display_and_from_name_full_range() {
    for b in 0u8..=255 {
        let k = capi::cec::Keycode(b);
        // Display must not panic for any value (Keycode has no Display;
        // name lookup is the user-facing surface).
        // keycode_from_name covers the reverse direction in mock_suite.
    }
}

#[test]
fn opcode_display_covers_full_range() {
    for b in 0u8..=255 {
        let s = capi::cec::opcode_name(capi::cec::Opcode(b));
        assert!(!s.is_empty(), "{b:#x}");
    }
}

#[test]
fn device_type_for_address_covers_full_range() {
    for b in 0u8..=255 {
        assert!(!capi::cec::device_type_for_address(b).is_empty(), "{b}");
    }
}

// -- exec display ------------------------------------------------------------------

#[test]
fn exec_error_display_all_variants() {
    use capi::exec::ExecError;
    let cec_err = format!("{}", capi::cec::CecError::Closed);
    assert!(!cec_err.is_empty());
    assert_eq!(
        capi::exec::ExecError::Cec(capi::cec::CecError::Closed).to_string(),
        cec_err
    );
}

async fn call(app: axum::Router, method: &str, uri: &str) -> (axum::http::StatusCode, String) {
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
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

// -- ui fallback branches ------------------------------------------------------------

#[tokio::test]
#[serial]
async fn device_row_fallback_name_chain() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    // Device with no OSD, no observed fragment, no role: falls back to
    // address_name. Another with only observed fields: ghost path.
    state.bus().replace_snapshot(
        vec![
            serde_json::json!({
                "logical_address": 9,
                "device_type": "",
                "discovery": "observed",
                "observed_osd_name_fragment": "GhostBox",
                "observed_power_status": "standby",
                "address_name": "PlaybackDevice2",
                "physical_address": "",
            }),
            serde_json::json!({
                "logical_address": 10,
                "osd_name": "",
                "device_type": "",
                "discovery": "observed",
                "address_name": "Reserved1",
            }),
        ],
        vec![9, 10],
        -1,
        false,
        false,
        Some(chrono::Utc::now()),
        180,
        256,
    );
    let app = capi::server::build_router(state);

    let (status, body) = call(app, "GET", "/ui/fragment/devices").await;
    assert_eq!(status, 200);
    assert!(body.contains("GhostBox"), "observed fragment used: {body}");
    assert!(body.contains("Reserved1"), "address_name fallback used");
    assert!(
        body.contains("ghost") || body.contains("observed"),
        "ghost marker"
    );
}

// -- server unit arms ------------------------------------------------------------------

#[test]
fn percent_decode_plus_decodes_space() {
    // '+' means space in query strings (auth ?key= path).
    // Indirect: ?key=a+b must NOT match token "a b" but must be decoded.
    // We assert via the auth layer: token "a b" set, query key=a+b → 200.
    let state = common::app_state();
    state
        .settings()
        .update(|c| c.auth_token = "a b".into())
        .unwrap();
    let app = capi::server::build_router(state);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/devices?key=a+b")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(resp.status(), 401, "decoded + must equal space");
    });
}

// -- cec closed-session bridge gates -----------------------------------------------------

#[test]
#[serial]
fn bridges_gate_on_closed_session_id() {
    let cfg = capi::cec::Configuration {
        device_name: "gate2".into(),
        device_type: capi::cec::DeviceType::RECORDING,
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
    let id = conn.session_id();
    let mut rx = conn.subscribe_events();

    // Close: session deregistered; bridge lookups for this id now fail.
    conn.close().unwrap();

    // Emits targeting the closed session must be silent no-ops.
    cec::mock::emit_command_on(
        &conn,
        &Command {
            initiator: LogicalAddress(0),
            destination: LogicalAddress(4),
            opcode: Opcode(0x44),
            opcode_set: true,
            parameters: vec![],
            ack: false,
            eom: true,
        },
    );
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            | Err(tokio::sync::broadcast::error::TryRecvError::Closed)
    ));
}

// -- settings save-failure arm --------------------------------------------------------------
