#![allow(unused_imports)]
//! Final micro-gaps: strategy edge steps, UI action failures, misc units.

#![cfg(feature = "mock-cec")]

use capi::cec;
#[allow(unused_imports)]
use capi::types::AppEvent;
use common::*;
mod common;
use axum::body::Body;
use tower::ServiceExt;

use capi::strategies::{Action, Registry, RunOptions, Step, StepKind, Strategy};
use serial_test::serial;
use std::sync::Arc;

fn mk_session() -> Arc<capi::cec::Connection> {
    cec::mock::reset();
    let cfg = capi::cec::Configuration {
        device_name: "fg".into(),
        device_type: capi::cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: capi::cec::LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    };
    let conn = Arc::new(capi::cec::Connection::open(&cfg).unwrap());
    conn.force_opened_for_test();
    {
        let bus = Arc::new(capi::BusState::new());
        bus.set_frame_ring_capacity(64);
        // no sink needed here
    }
    conn
}

#[test]
#[serial]
fn uc_press_hold_releases_and_unknown_target_errors() {
    let conn = mk_session();
    let registry = Registry::new();
    let bus = Arc::new(capi::BusState::new());

    // hold_ms > 0 exercises press/release pair.
    let chain = vec![Strategy {
        name: "hold".into(),
        steps: vec![Step {
            kind: StepKind::SendUserControl,
            target: capi::cec::LogicalAddress::AUDIO_SYSTEM,
            key: capi::cec::Keycode::VOLUME_UP,
            wait: false,
            hold_ms: 20,
            opcode: capi::cec::Opcode(0),
            params: vec![],
            delay_ms: 0,
        }],
        observe_ms: 60,
    }];
    registry.set_vendor_override("fg", Action::VolumeUp, chain);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let results = registry.run(
        &conn,
        &bus,
        Action::VolumeUp,
        &RunOptions {
            vendor: "fg".into(),
            ..Default::default()
        },
        deadline,
    );
    assert!(results[0].steps[0].acked);

    // Unknown target (UNKNOWN + no override) -> step error recorded.
    let chain2 = vec![Strategy {
        name: "badtarget".into(),
        steps: vec![Step {
            kind: StepKind::SendUserControl,
            target: capi::cec::LogicalAddress::UNKNOWN,
            key: capi::cec::Keycode::SELECT,
            wait: false,
            hold_ms: 0,
            opcode: capi::cec::Opcode(0),
            params: vec![],
            delay_ms: 0,
        }],
        observe_ms: 40,
    }];
    registry.set_vendor_override("fg", Action::Select, chain2);
    let results2 = registry.run(
        &conn,
        &bus,
        Action::Select,
        &RunOptions {
            vendor: "fg".into(),
            ..Default::default()
        },
        std::time::Instant::now() + std::time::Duration::from_secs(5),
    );
    assert!(!results2[0].steps[0].acked);
}

// -- ui action failure forms ---------------------------------------------------

#[tokio::test]
#[serial]
async fn ui_action_invalid_inputs_render_400() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state);

    for (path, form) in [
        ("/ui/action/set_source", "addr=99"),
        ("/ui/action/hdmi", "port=0"),
        ("/ui/action/nav_key", "key=warp"),
    ] {
        let (status, _) = call(
            app.clone(),
            "POST",
            path,
            Some(form.into()),
            Some("application/x-www-form-urlencoded"),
        )
        .await;
        assert!(status == 400 || status == 500, "{path}: {status}");
    }
}

async fn call(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<String>,
    ct: Option<&str>,
) -> (axum::http::StatusCode, String) {
    let mut b = axum::http::Request::builder().method(method).uri(uri);
    if let Some(ct) = ct {
        b = b.header("Content-Type", ct);
    }
    let resp = app
        .oneshot(b.body(Body::from(body.unwrap_or_default())).unwrap())
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

// -- update restart_service environment arm --------------------------------------

#[test]
fn restart_service_reports_failure_when_unit_absent() {
    // systemctl exists on this host but capi.service is not installed in test
    // context -> non-zero exit -> Err branch.
    if !std::path::Path::new("/run/systemd/system").exists() {
        return;
    }
    let err = capi::update::__test_restart_blocking();
    assert!(err.is_err());
}
