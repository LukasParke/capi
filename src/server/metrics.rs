//! Prometheus-style text metrics from in-process counters.

use super::AppState;
use axum::extract::State;
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use std::sync::atomic::Ordering;

pub async fn metrics_handler(State(state): State<AppState>) -> Response {
    let m = &state.0.metrics;
    let (dropped, delivered) = state.0.hub.stats();
    let (queued, dropped_jobs) = state.0.steward.counters();
    let frames = state.0.bus.frames_captured.load(Ordering::Relaxed);
    let body = format!(
        "# HELP capi_requests_total Total HTTP requests.\n\
         # TYPE capi_requests_total counter\n\
         capi_requests_total {}\n\
         # HELP capi_errors_total Total non-2xx responses.\n\
         # TYPE capi_errors_total counter\n\
         capi_errors_total {}\n\
         # HELP capi_panics_total Recovered handler panics.\n\
         # TYPE capi_panics_total counter\n\
         capi_panics_total {}\n\
         # HELP capi_events_published_total Events published to the hub.\n\
         # TYPE capi_events_published_total counter\n\
         capi_events_published_total {}\n\
         # HELP capi_events_dropped_total Hub events dropped (slow subscribers).\n\
         # TYPE capi_events_dropped_total counter\n\
         capi_events_dropped_total {dropped}\n\
         # HELP capi_events_delivered_total Hub events delivered.\n\
         # TYPE capi_events_delivered_total counter\n\
         capi_events_delivered_total {delivered}\n\
         # HELP capi_steward_jobs_queued_total Steward jobs executed.\n\
         # TYPE capi_steward_jobs_queued_total counter\n\
         capi_steward_jobs_queued_total {queued}\n\
         # HELP capi_steward_jobs_dropped_total Steward jobs dropped (queue full).\n\
         # TYPE capi_steward_jobs_dropped_total counter\n\
         capi_steward_jobs_dropped_total {dropped_jobs}\n\
         # HELP capi_frames_captured_total CEC frames captured in the ring.\n\
         # TYPE capi_frames_captured_total counter\n\
         capi_frames_captured_total {frames}\n\
         # HELP capi_subscribers Active SSE/WS/MQTT subscribers.\n\
         # TYPE capi_subscribers gauge\n\
         capi_subscribers {}\n\
         # HELP capi_adapter_ready Whether a live adapter session exists.\n\
         # TYPE capi_adapter_ready gauge\n\
         capi_adapter_ready {}\n",
        m.requests_total.load(Ordering::Relaxed),
        m.errors_total.load(Ordering::Relaxed),
        m.panics_total.load(Ordering::Relaxed),
        m.events_published.load(Ordering::Relaxed),
        state.0.hub.subscriber_count(),
        if state.0.adapter.ready() { 1 } else { 0 },
    );
    let mut resp = body.into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4"),
    );
    resp
}
