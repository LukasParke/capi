#![cfg(feature = "mock-cec")]
//! Streaming-surface tests: SSE body semantics (immediate buffered event,
//! keepalive) and the WebSocket OOB coalescer, driven over a REAL listener.

use serial_test::serial;
mod common;

use capi::server;
use common::*;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;

async fn spawn_app(state: server::AppState) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = server::build_router(state);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), handle)
}

#[tokio::test]
#[serial]
async fn sse_delivers_buffered_event_immediately() {
    use axum::body::Body;
    use axum::http::Request;
    use futures_util::StreamExt;
    use tower::ServiceExt;

    let state = app_state();
    let app = server::build_router(state.clone());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_success());
    assert!(resp.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));

    // First body frame must carry the named event. The handler subscribed
    // when the response was created, so publishing now is deliverable;
    // read with a timeout because the stream intentionally never ends.
    state.hub().publish(capi::types::AppEvent::new(
        "power_change",
        serde_json::json!({"address": 0, "status": "standby"}),
    ));
    let mut stream = resp.into_body().into_data_stream();
    let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("first frame within timeout")
        .expect("stream yields first frame")
        .expect("frame payload ok");
    let text = String::from_utf8_lossy(&chunk);
    assert!(text.contains("event: power_change"), "{text}");
    assert!(text.contains("\"address\":0"), "{text}");
}

#[tokio::test]
#[serial]
async fn ws_oob_stream_renders_coalesced_fragments() {
    let state = app_state_with_monitor_session();
    // Seed a device so panel renders have content.
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
    let (base, _srv) = spawn_app(state.clone()).await;
    let ws_url = format!("{}/api/events/ws", base.replacen("http", "ws", 1));

    let (mut ws, _resp) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("ws connect");

    // Trigger events that mark panels dirty.
    state.hub().publish(capi::types::AppEvent::new(
        "devices_changed",
        serde_json::json!({"reason": "test", "logical_addresses": [4]}),
    ));
    state.hub().publish(capi::types::AppEvent::new(
        "source_activated",
        serde_json::json!({"address": 4, "activated": true}),
    ));

    // Coalescer flushes within ~120ms; feed line goes out immediately.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut got_feed = false;
    let mut got_oob = false;
    while tokio::time::Instant::now() < deadline && !(got_feed && got_oob) {
        let msg = tokio::time::timeout(Duration::from_secs(3), ws.next()).await;
        match msg {
            Ok(Some(Ok(m))) if m.is_text() => {
                let text = m.into_text().unwrap();
                if text.contains("feed-line") || text.contains("data-kind") {
                    got_feed = true;
                }
                if text.contains("hx-swap-oob") && text.contains("devices-panel") {
                    got_oob = true;
                }
            }
            Ok(Some(Ok(m))) if m.is_ping() => {
                ws.send(tungstenite_pong()).await.ok();
            }
            Ok(Some(Err(e))) => panic!("ws error: {e}"),
            Ok(Some(Ok(_))) => {}
            Ok(None) => break,
            Err(e) => panic!("ws timeout: {e}"),
        }
    }
    assert!(got_feed, "immediate feed line received");
    assert!(got_oob, "coalesced OOB panel fragment received");
}

fn tungstenite_pong() -> tokio_tungstenite::tungstenite::Message {
    tokio_tungstenite::tungstenite::Message::Pong(vec![])
}

#[tokio::test]
#[serial]
async fn ws_rejects_foreign_origin() {
    let state = app_state();
    let (base, _srv) = spawn_app(state).await;
    let ws_url = format!("{}/api/events/ws", base.replacen("http", "ws", 1));
    let req =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(&ws_url)
            .unwrap();
    let (req, _) = {
        let mut r = req;
        r.headers_mut()
            .insert("Origin", "https://evil.example".parse().unwrap());
        (r, ())
    };
    let result = tokio_tungstenite::connect_async(req).await;
    assert!(result.is_err(), "foreign origin must be rejected");
}
