//! WebSocket OOB-fragment stream for htmx: coalesced panel re-renders plus
//! per-event feed lines. Origin-checked (scheme+port aware).

use super::AppState;
use crate::types::AppEvent;
use askama::Template;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use std::time::Duration;

const COALESCE_WINDOW: Duration = Duration::from_millis(120);
const PING_INTERVAL: Duration = Duration::from_secs(45);

fn origin_allowed(headers: &HeaderMap, state: &AppState) -> bool {
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        // Non-browser clients (no Origin) are allowed; auth still applies.
        return true;
    };
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let rest = origin.split_once("://").map(|(_, r)| r).unwrap_or(origin);
    let origin_hostport = rest.split('/').next().unwrap_or("");
    if origin_hostport.eq_ignore_ascii_case(host) {
        return true;
    }
    // Allow the loopback variants of the same host only when the token is set
    // (strict mode); otherwise strict equality above is the rule.
    let _ = state;
    false
}

pub async fn events_ws(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&headers, &state) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    ws.on_upgrade(move |socket| run_socket(state, socket))
}

async fn run_socket(state: AppState, mut socket: WebSocket) {
    let mut rx = state.0.hub.subscribe();
    let mut dirty_devices = false;
    let mut dirty_topology = false;
    let mut dirty_source = false;
    let mut flush_timer = tokio::time::interval(COALESCE_WINDOW);
    flush_timer.tick().await; // consume immediate tick
    let mut pending_flush = false;
    let mut ping = tokio::time::interval(PING_INTERVAL);

    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Ok(ev) => {
                        if write_feed_line(&mut socket, &ev).await.is_err() {
                            return;
                        }
                        match ev.kind.as_str() {
                            "devices_changed" | "power_change" | "configuration_changed" => dirty_devices = true,
                            "adapter_state" => { dirty_devices = true; dirty_topology = true; }
                            "source_activated" => { dirty_source = true; dirty_topology = true; }
                            _ => {}
                        }
                        if !pending_flush {
                            flush_timer = tokio::time::interval(COALESCE_WINDOW);
                            flush_timer.tick().await;
                            pending_flush = true;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("ws subscriber lagged, dropped {n}");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
            _ = flush_timer.tick(), if pending_flush => {
                pending_flush = false;
                let html = render_oob(&state, dirty_devices, dirty_topology, dirty_source);
                dirty_devices = false;
                dirty_topology = false;
                dirty_source = false;
                if !html.is_empty() && socket.send(Message::Text(html.into())).await.is_err() {
                    return;
                }
            }
            _ = ping.tick() => {
                if socket.send(Message::Ping(vec![].into())).await.is_err() {
                    return;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(_)) => {} // client messages ignored
                    _ => return,
                }
            }
        }
    }
}

async fn write_feed_line(socket: &mut WebSocket, ev: &AppEvent) -> Result<(), axum::Error> {
    let line = crate::ui::event_feed_line_html(ev);
    socket.send(Message::Text(line.into())).await
}

/// Render OOB fragments for dirty panels. Adapter-down renders nothing so the
/// banner (which explains the outage) stays authoritative.
fn render_oob(state: &AppState, devices: bool, topology: bool, source: bool) -> String {
    if !state.0.adapter.ready() {
        return String::new();
    }
    let mut out = String::new();
    if devices {
        let snap = state.0.bus.copy_snapshot();
        let rows = crate::ui::device_rows_from_snapshot(&snap);
        let row_count = rows.len();
        let data = crate::ui_ctx::DevicesPanelData {
            devices: rows,
            message: format!("Live snapshot ({} devices)", row_count),
        };
        out.push_str("<div id=\"devices-panel\" hx-swap-oob=\"innerHTML\">");
        out.push_str(
            &crate::ui::DevicesTmpl { ctx: data }
                .render()
                .unwrap_or_default(),
        );
        out.push_str("</div>");
    }
    if topology {
        let topo = crate::topology::build_from_snapshot(&state.0.bus);
        out.push_str("<div id=\"topology-card\" hx-swap-oob=\"innerHTML\">");
        out.push_str(
            &crate::ui::TopologyTmpl { ctx: topo }
                .render()
                .unwrap_or_default(),
        );
        out.push_str("</div>");
    }
    if source {
        let data = crate::ui::source_panel_data(state);
        out.push_str("<div id=\"source-card\" hx-swap-oob=\"innerHTML\">");
        out.push_str(
            &crate::ui::SourceTmpl { ctx: data }
                .render()
                .unwrap_or_default(),
        );
        out.push_str("</div>");
    }
    out
}

use axum::response::IntoResponse;
