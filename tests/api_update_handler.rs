//! /api/update handler arms with a local mock GitHub (feature mock-cec).

#![cfg(feature = "mock-cec")]

mod common;

use axum::body::Body;
use common::*;
use serial_test::serial;
use tower::ServiceExt;

#[tokio::test]
#[serial]
async fn update_handler_already_current_is_200() {
    std::env::set_var("CAPI_UPDATE_BASE_TEST", "http://127.0.0.1:1"); // unused here
    let state = app_state_with_live_session();

    // Serve "already current" from a local mock and point the seam at it.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let ver = capi::ui::VERSION.to_string();
    let latest = serde_json::json!({"tag_name": ver, "assets": []});
    let app2 = axum::Router::new().route(
        "/repos/LukasParke/capi/releases/latest",
        axum::routing::get(move || {
            let body = latest.clone();
            async move { axum::Json(body) }
        }),
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app2).await;
    });
    std::env::set_var("CAPI_UPDATE_BASE_TEST", format!("http://{addr}"));

    let app = capi::server::build_router(state);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/update")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["message"], "Already up to date");
    std::env::remove_var("CAPI_UPDATE_BASE_TEST");
}

#[tokio::test]
#[serial]
async fn update_handler_missing_sums_is_502() {
    let state = app_state_with_live_session();
    let app = capi::server::build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let missing_sums = serde_json::json!({
        "tag_name": "v99.0.0",
        "assets": [
            {"name": "capi-linux-arm64-libcec6", "browser_download_url": format!("{base}/assets/bin")},
        ],
    });
    let app2 = axum::Router::new()
        .route(
            "/repos/LukasParke/capi/releases/latest",
            axum::routing::get(move || {
                let body = missing_sums.clone();
                async move { axum::Json(body) }
            }),
        )
        .route("/assets/bin", axum::routing::get(|| async { vec![1u8] }));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app2).await;
    });
    std::env::set_var("CAPI_UPDATE_BASE_TEST", format!("http://{addr}"));

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/update")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 502);
    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // On x86_64 hosts the arch guard fires before the sums check; either way
    // the handler must surface a 502 envelope with an explanatory message.
    assert!(!v["message"].as_str().unwrap().is_empty());
    std::env::remove_var("CAPI_UPDATE_BASE_TEST");
}
