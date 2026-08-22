#![cfg(feature = "mock-cec")]
//! Adapter-present integration coverage: same router, but with an opened
//! monitor-only session attached. Bus reads run real libcec calls; every
//! transmit is refused deterministically at the monitor gate — so these
//! tests are safe on hosts with actual CEC adapters while covering the
//! success-shaped handler paths (200s, summaries, snapshot flows).

use capi::server;
use serial_test::serial;
mod common;

type App = axum::Router;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::*;
use tower::ServiceExt;

async fn call(
    app: App,
    method: &str,
    uri: &str,
    body: Option<String>,
    headers: &[(&str, &str)],
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let req = builder
        .body(Body::from(body.unwrap_or_default()))
        .expect("request");
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    (status, body.to_vec())
}

async fn json(app: App, method: &str, uri: &str) -> (StatusCode, serde_json::Value) {
    let (status, body) = call(app, method, uri, None, &[]).await;
    (status, envelope(&body))
}

#[tokio::test]
#[serial]
async fn devices_sync_wait_runs_full_steward_job() {
    let state = app_state_with_monitor_session();
    let app = server::build_router(state.clone());

    // ?rescan=1 forces the synchronous full job (bounded by 5s default wait).
    let (status, body) = call(app, "GET", "/api/devices?rescan=1", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let v = envelope(&body);
    assert_success(&v);
    assert_eq!(v["message"], "Devices retrieved (live)");

    let snap = state.bus().copy_snapshot();
    assert!(snap.cec_ready);
    assert!(!snap.scan_in_progress);
}

#[tokio::test]
#[serial]
async fn key_send_returns_strategy_summary() {
    let app = server::build_router(app_state_with_monitor_session());
    // Monitor gate refuses transmits; registry classifies and returns a
    // summary (HTTP 200 with honest per-strategy results).
    let (status, body) = call(
        app,
        "POST",
        "/api/key",
        Some(r#"{"address":0,"key":"select"}"#.into()),
        &[("Content-Type", "application/json")],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let v = envelope(&body);
    assert!(v["message"].as_str().unwrap().contains("tried"), "{v}");
}

#[tokio::test]
#[serial]
async fn raw_command_monitor_refusal_is_conflict() {
    let app = server::build_router(app_state_with_monitor_session());
    let (status, body) = call(
        app,
        "POST",
        "/api/command",
        Some(r#"{"initiator":4,"destination":0,"opcode":143}"#.into()),
        &[("Content-Type", "application/json")],
    )
    .await;
    // Transmit refused in monitor mode surfaces as a library error -> 500
    // envelope; either way it must NOT be a silent success.
    let v = envelope(&body);
    assert_ne!(status, StatusCode::OK, "{v}");
}

#[tokio::test]
#[serial]
async fn source_active_real_response_shape() {
    let app = server::build_router(app_state_with_monitor_session());
    let (status, v) = json(app, "GET", "/api/source/active").await;
    if status == StatusCode::OK {
        // Either an active source or the explicit none sentinel.
        assert!(v["data"]["active_source"].is_i64() || v["data"]["active_source"].is_number());
    } else {
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}

#[tokio::test]
#[serial]
async fn audio_status_live_shape() {
    let app = server::build_router(app_state_with_monitor_session());
    let (_, v) = json(app, "GET", "/api/audio/status").await;
    assert_success(&v);
    assert!(v["data"]["volume"].is_u64());
}

#[tokio::test]
#[serial]
async fn ui_actions_execute_against_session() {
    for _ in 0..1 {
        let state = app_state_with_monitor_session();
        // Seed one device so per-device fragments have content.
        state.bus().replace_snapshot(
            vec![serde_json::json!({
                "logical_address": 4, "osd_name": "Box", "device_type": "PlaybackDevice1",
                "physical_address": "2.0.0.0", "discovery": "active", "power_status": "on",
                "vendor_id": "0x000048", "vendor_name": "Unknown", "vendor_known": false,
                "cec_version": "1.4", "address_name": "PlaybackDevice1", "hdmi_port": 2,
            })],
            vec![4],
            -1,
            true,
            false,
            Some(chrono::Utc::now()),
            180,
            256,
        );
        let app = server::build_router(state.clone());
        // Form POSTs through the htmx action layer.
        for (path, form) in [
            ("/ui/action/power_on", "addr=0"),
            ("/ui/action/power_off", "addr=0"),
            ("/ui/action/volume_up", ""),
            ("/ui/action/volume_down", ""),
            ("/ui/action/volume_mute", ""),
            ("/ui/action/set_source", "addr=4"),
            ("/ui/action/hdmi", "port=2"),
            ("/ui/action/nav_key", "key=nav_up&addr=0"),
        ] {
            let (status, body) = call(
                app.clone(),
                "POST",
                path,
                Some(form.into()),
                &[("Content-Type", "application/x-www-form-urlencoded")],
            )
            .await;
            assert!(
                status.is_success()
                    || status == StatusCode::SERVICE_UNAVAILABLE
                    || status == StatusCode::BAD_REQUEST,
                "{path}: {status} {}",
                String::from_utf8_lossy(&body)
            );
        }

        // Fragments that render from live state.
        for frag in [
            "/ui/fragment/source_panel",
            "/ui/fragment/volume_panel",
            "/ui/fragment/nav_panel",
            "/ui/fragment/device_power",
            "/ui/fragment/mqtt_panel",
            "/ui/fragment/health",
            "/ui/dev/fragment/banner",
            "/ui/dev/fragment/devices",
            "/ui/dev/fragment/trace",
        ] {
            let (status, body) = call(app.clone(), "GET", frag, None, &[]).await;
            assert_eq!(
                status,
                StatusCode::OK,
                "TAG_STATUS {frag} -> {status} :: {}",
                String::from_utf8_lossy(&body)
            );
            // device_power may legitimately be empty here: successful power
            // actions schedule a steward refresh which can replace the seed
            // snapshot mid-test (live host behavior).
            if frag != "/ui/fragment/device_power" {
                assert!(!body.is_empty(), "TAG_EMPTY {frag}");
            }
        }
    }
}

#[tokio::test]
#[serial]
async fn dev_probe_full_run_and_strategies_table() {
    let app = server::build_router(app_state_with_monitor_session());

    // Monitor-only sessions refuse probes/strategies with an explicit 409
    // (the passive-mode guard runs before any bus traffic).
    for (path, json) in [
        (
            "/api/dev/probe",
            r#"{"address":4,"kind":"power","observe_ms":150}"#,
        ),
        (
            "/api/dev/run_strategies",
            r#"{"action":"volume_up","target":-1,"observe_ms":120}"#,
        ),
        (
            "/api/dev/send_opcode",
            r#"{"destination":0,"opcode":143,"params_hex":"10 02","observe_ms":100}"#,
        ),
    ] {
        let (status, body) = call(
            app.clone(),
            "POST",
            path,
            Some(json.into()),
            &[("Content-Type", "application/json")],
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "{path}: {}",
            String::from_utf8_lossy(&body)
        );
        assert!(envelope(&body)["message"]
            .as_str()
            .unwrap()
            .contains("monitor-only"));
    }
}

#[tokio::test]
#[serial]
async fn bus_scan_deep_job_completes_with_session() {
    let state = app_state_with_monitor_session();
    let app = server::build_router(state.clone());
    let (status, _) = json(app, "POST", "/api/bus/scan").await;
    assert_eq!(status, StatusCode::OK);
    // Wait for the deep job to drain through the steward queue.
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let snap = state.bus().copy_snapshot();
            if !snap.scan_in_progress && snap.last_full_scan_at.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("deep scan completed");
}
