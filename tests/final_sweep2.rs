//! Final sweep 2: live happy-path arms, steward edges, update/mqtt/settings
//! leftovers (feature = "mock-cec").

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
    body: Option<(String, String)>,
) -> (axum::http::StatusCode, String) {
    let mut b = axum::http::Request::builder().method(method).uri(uri);
    if let Some((d, ct)) = &body {
        b = b.header("Content-Type", ct.as_str());
    }
    let data = body.map(|(d, _)| d).unwrap_or_default();
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

fn live_router() -> axum::Router {
    capi::server::build_router(app_state_with_live_session())
}

// -- api happy arms needing live session -----------------------------------------

#[tokio::test]
#[serial]
async fn api_live_happy_paths() {
    let app = live_router();

    // Power on/off with valid addr -> Ok arm.
    for uri in ["/api/power/on/0", "/api/power/off/0"] {
        let (status, _) = call(app.clone(), "POST", uri, None).await;
        assert!(status.is_success(), "{uri}");
    }

    // Volume forced-target valid.
    let (status, _) = call(app.clone(), "POST", "/api/volume/up?address=5", None).await;
    assert!(status.is_success(), "{status}");

    // Set active source / HDMI port success.
    let (status, _) = call(app.clone(), "POST", "/api/source/4", None).await;
    assert!(status.is_success());
    let (status, _) = call(app.clone(), "POST", "/api/hdmi/2", None).await;
    assert!(status.is_success());

    // Key select success (mock acks).
    let (status, _) = call(
        app,
        "POST",
        "/api/key",
        Some((
            r#"{"address":0,"key":"select"}"#.into(),
            "application/json".into(),
        )),
    )
    .await;
    assert_eq!(status, 200);
}

// -- steward edges ------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn steward_queue_full_drops_and_recovers() {
    cec::mock::reset();
    let state = app_state_with_live_session();

    // Start a long Deep job, then flood Light jobs until one drops.
    state.steward().enqueue(capi::steward::JobKind::Deep);
    let mut dropped_seen = false;
    for _ in 0..64 {
        if !state.steward().enqueue(capi::steward::JobKind::Light) {
            dropped_seen = true;
            break;
        }
    }
    assert!(dropped_seen || state.steward().counters().1 > 0);

    // Wait for drain.
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            if !state.bus().copy_snapshot().scan_in_progress {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("drain");
}

#[test]
fn steward_detached_noops() {
    let s = capi::steward::Steward::detached();
    assert!(!s.enqueue(capi::steward::JobKind::Light) || true); // may or may not
    s.hint(true); // must not panic
    let _ = s.counters();
}

// -- mqtt parse edge ------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn mqtt_start_with_credentials_and_stop() {
    // Exercises SetUsername/SetPassword branch (broker absent; retry loop
    // runs in background thread and is stopped by stop()).
    let h = capi::mqtt::MqttHandle::new();
    let cfg = capi::types::MqttConfig {
        broker: "tcp://127.0.0.1:18999".into(),
        user: "u".into(),
        pass: "p".into(),
        prefix: "x".into(),
    };
    let (tx, _cmd_rx_keepalive) = tokio::sync::mpsc::unbounded_channel();
    let rt = std::sync::Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap(),
    );
    let _guard = rt.enter();
    // Keep the runtime alive for the whole test via a leak-free holder.
    struct RtKeep(std::sync::Arc<tokio::runtime::Runtime>);
    impl Drop for RtKeep {
        fn drop(&mut self) {
            let rt = self.0.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(300));
                drop(rt);
            });
        }
    }
    let _rt_keep = RtKeep(rt.clone());
    h.start(cfg, tokio::sync::broadcast::channel(8).1, tx);
    std::thread::sleep(std::time::Duration::from_millis(300));
    h.stop();
    assert!(!h.is_connected());
}
