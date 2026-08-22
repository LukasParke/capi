#![cfg(feature = "mock-cec")]
//! UI surface sweep: every GET fragment and POST action against both
//! monitor-only and live mock sessions (feature = "mock-cec").

#![cfg(feature = "mock-cec")]

use capi::cec;
mod common;

use axum::body::Body;
use common::*;
use serial_test::serial;
use tower::ServiceExt;

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

#[tokio::test]
#[serial]
async fn all_get_fragments_render() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    state.bus().replace_snapshot(
        vec![serde_json::json!({
            "logical_address": 4, "osd_name": "Box", "device_type": "PlaybackDevice1",
            "physical_address": "2.0.0.0", "discovery": "active", "power_status": "on",
            "vendor_id": "0x000048", "vendor_name": "Unknown", "vendor_known": false,
            "cec_version": "1.4", "address_name": "PlaybackDevice1", "hdmi_port": 2,
        })],
        vec![4],
        4,
        true,
        false,
        Some(chrono::Utc::now()),
        180,
        256,
    );
    let app = capi::server::build_router(state);

    for frag in [
        "/ui/fragment/bus_banner",
        "/ui/fragment/devices",
        "/ui/fragment/device_power",
        "/ui/fragment/mqtt_panel",
        "/ui/fragment/health",
        "/ui/fragment/topology_hdmi",
        "/ui/fragment/volume_panel",
        "/ui/fragment/nav_panel",
        "/ui/fragment/source_panel",
        "/ui/fragment/logs",
    ] {
        let (status, body) = call(app.clone(), "GET", frag, None, None).await;
        assert_eq!(status, 200, "{frag}");
        assert!(!body.is_empty(), "{frag} must render");
    }
}

#[tokio::test]
#[serial]
async fn all_post_actions_execute() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state.clone());

    let actions = [
        ("/ui/action/power_on", "addr=0"),
        ("/ui/action/power_off", "addr=0"),
        ("/ui/action/volume_up", ""),
        ("/ui/action/volume_down", ""),
        ("/ui/action/volume_mute", ""),
        ("/ui/action/set_source", "addr=4"),
        ("/ui/action/hdmi", "port=2"),
        ("/ui/action/nav_key", "key=nav_up&addr=0"),
        ("/ui/action/nav_key", "key=back"),
        ("/ui/action/deep_scan", ""),
        ("/ui/action/mqtt_save", "broker=&user=&pass=&prefix="),
    ];
    for (path, form) in actions {
        let (status, _body) = call(
            app.clone(),
            "POST",
            path,
            Some(form.into()),
            Some("application/x-www-form-urlencoded"),
        )
        .await;
        assert!(
            status.is_success() || status == 503 || status == 400,
            "{path}: {status}"
        );
    }
}

#[tokio::test]
#[serial]
async fn login_page_renders() {
    cec::mock::reset();
    let app = capi::server::build_router(app_state_with_live_session());
    let (status, body) = call(app, "GET", "/login", None, None).await;
    assert_eq!(status, 200);
    assert!(body.contains("token"), "login form present");
}

#[tokio::test]
#[serial]
async fn dev_pages_render_with_session() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state);

    let (status, body) = call(app.clone(), "GET", "/dev", None, None).await;
    assert_eq!(status, 200);
    assert!(
        body.contains("Dev console") || body.contains("dev"),
        "{body}"
    );

    for frag in [
        "/ui/dev/fragment/banner",
        "/ui/dev/fragment/devices",
        "/ui/dev/fragment/trace",
    ] {
        let (status, body) = call(app.clone(), "GET", frag, None, None).await;
        assert_eq!(status, 200, "{frag}");
        let _ = body;
    }
}
