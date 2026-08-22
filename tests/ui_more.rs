#![cfg(feature = "mock-cec")]
//! Remaining UI/api arms: legacy fragments, login failure render, feed-line
//! kinds via the real handler, remote data edges (feature = "mock-cec").

#![cfg(feature = "mock-cec")]

use axum::body::Body;
use capi::cec;
mod common;

use capi::types::AppEvent;
use common::*;
use serial_test::serial;
use tower::ServiceExt;

#[tokio::test]
#[serial]
async fn login_wrong_token_renders_error() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    state
        .settings()
        .update(|c| c.auth_token = "tok".into())
        .unwrap();
    capi::ui::LOGIN_TOKEN.set("tok".into()).ok();
    let app = capi::server::build_router(state);

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/login")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from("token=wrong"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("Invalid token"));
}

#[test]
#[serial]
fn event_feed_line_all_kinds_render() {
    cec::mock::reset();
    for kind in [
        capi::types::event_type::POWER_CHANGE,
        capi::types::event_type::SOURCE_ACTIVATED,
        capi::types::event_type::KEY_PRESS,
        capi::types::event_type::COMMAND,
        capi::types::event_type::ALERT,
        capi::types::event_type::DEVICES_CHANGED,
        capi::types::event_type::CONFIGURATION_CHANGED,
        capi::types::event_type::ADAPTER_STATE,
    ] {
        let html = capi::ui::event_feed_line_html(&AppEvent::new(kind, serde_json::json!({})));
        assert!(html.contains("feed-line"), "{kind}");
    }
}
