#![cfg(feature = "mock-cec")]
//! Negative/edge coverage for dev_api and api handlers (mock feature).

#![cfg(feature = "mock-cec")]

use capi::cec;
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

#[tokio::test]
#[serial]
async fn dev_probe_invalid_json_and_kind() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state);

    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/dev/probe")
                .header("Content-Type", "application/json")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // observe_ms clamped, kind filtered to none -> 400 unknown kind.
    let (status, v) = post_json(
        app.clone(),
        "/api/dev/probe",
        r#"{"address":0,"kind":"nonexistent"}"#,
    )
    .await;
    assert_eq!(status, 400);
    assert!(v["message"]
        .as_str()
        .unwrap()
        .contains("unknown probe kind"));
}

#[tokio::test]
#[serial]
async fn dev_send_opcode_bad_hex() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state);
    for bad in ["ZZ", "1"] {
        let (status, v) = post_json(
            app.clone(),
            "/api/dev/send_opcode",
            &format!(r#"{{"destination":0,"opcode":1,"params_hex":"{bad}"}}"#),
        )
        .await;
        assert_eq!(status, 400, "{bad}: {v}");
    }
}

#[tokio::test]
#[serial]
async fn dev_save_strategy_unknown_action_400() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state);
    let (status, v) = post_json(
        app,
        "/api/dev/save_strategy",
        r#"{"vendor":"0x1","action":"warp","strategy":"s"}"#,
    )
    .await;
    assert_eq!(status, 400);
    assert!(v["message"].as_str().unwrap().contains("unknown action"));
}

#[tokio::test]
#[serial]
async fn api_method_mismatches_are_405() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state);
    // GET on a POST-only route.
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/power/on")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
}

#[tokio::test]
#[serial]
async fn api_unknown_route_404() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
#[serial]
async fn key_send_unsupported_key_400_live() {
    cec::mock::reset();
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state);
    let (status, _) = post_json(app, "/api/key", r#"{"address":0,"key":"warp_drive"}"#).await;
    assert_eq!(status, 400);
}
