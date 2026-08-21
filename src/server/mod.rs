//! HTTP server: state, router, middleware (request-id, recovery, logging,
//! auth/CSRF), graceful shutdown.

pub mod api;
pub mod dev_api;
pub mod metrics;
pub mod sse;
pub mod ws;

use crate::adapter::AdapterHandle;
use crate::busstate::BusState;
use crate::events::{EventHub, LogRing, Metrics};
use crate::settings::Settings;
use crate::steward::Steward;
use crate::strategies::Registry;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::middleware::{self as axmw, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub settings: Arc<Settings>,
    pub hub: Arc<EventHub>,
    pub logs: Arc<LogRing>,
    pub bus: Arc<BusState>,
    pub adapter: AdapterHandle,
    pub steward: Arc<Steward>,
    pub registry: Arc<Registry>,
    pub metrics: Arc<Metrics>,
    pub mqtt: crate::mqtt::MqttHandle,
    pub started: chrono::DateTime<chrono::Utc>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        settings: Arc<Settings>,
        hub: Arc<EventHub>,
        logs: Arc<LogRing>,
        bus: Arc<BusState>,
        adapter: AdapterHandle,
        steward: Arc<Steward>,
        registry: Arc<Registry>,
        metrics: Arc<Metrics>,
        mqtt: crate::mqtt::MqttHandle,
    ) -> Self {
        Self(Arc::new(Inner {
            settings,
            hub,
            logs,
            bus,
            adapter,
            steward,
            registry,
            metrics,
            mqtt,
            started: chrono::Utc::now(),
        }))
    }

    pub fn auth_token(&self) -> String {
        self.0.settings.get().auth_token
    }

    pub fn mqtt_connected(&self) -> bool {
        self.0.mqtt.is_connected()
    }

    /// Apply a new MQTT config: start when broker set, stop otherwise.
    pub fn apply_mqtt_config(&self, cfg: &crate::types::MqttConfig) {
        if cfg.broker.is_empty() {
            self.0.mqtt.stop();
        } else {
            self.0
                .mqtt
                .start(cfg.clone(), self.0.hub.subscribe(), mqtt_cmd_tx());
        }
    }
}

pub fn mqtt_cmd_tx() -> tokio::sync::mpsc::UnboundedSender<crate::mqtt::MqttCommand> {
    MQTT_CMD_TX
        .get()
        .cloned()
        .expect("mqtt command channel initialized at boot")
}

pub static MQTT_CMD_TX: std::sync::OnceLock<
    tokio::sync::mpsc::UnboundedSender<crate::mqtt::MqttCommand>,
> = std::sync::OnceLock::new();

pub fn truthy(s: &str) -> bool {
    matches!(
        s.trim().to_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub fn api_map_exec(e: &crate::exec::ExecError) -> Response {
    match e {
        crate::exec::ExecError::AdapterUnavailable => unavailable(),
        crate::exec::ExecError::InvalidLogicalAddress
        | crate::exec::ExecError::InvalidHdmiPort
        | crate::exec::ExecError::InvalidKey => err(StatusCode::BAD_REQUEST, e.to_string()),
        _ => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// JSON envelope responses.
pub fn ok(message: impl Into<String>, data: Option<serde_json::Value>) -> Response {
    let env = crate::types::Envelope::success(message, data);
    json_response(StatusCode::OK, &env)
}

pub fn accepted(message: impl Into<String>, data: Option<serde_json::Value>) -> Response {
    let env = crate::types::Envelope::success(message, data);
    json_response(StatusCode::OK, &env)
}

pub fn err(status: StatusCode, message: impl Into<String>) -> Response {
    let env = crate::types::Envelope::error(message);
    json_response(status, &env)
}

pub fn json_response(status: StatusCode, env: &crate::types::Envelope) -> Response {
    let body = serde_json::to_string(env).unwrap_or_else(|_| r#"{"status":"error"}"#.into());
    let mut resp = (status, body).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp
}

pub fn unavailable() -> Response {
    err(StatusCode::SERVICE_UNAVAILABLE, "CEC adapter not available")
}

static REQUEST_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Request-id + access log + panic recovery + metrics counters.
async fn observability(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // Sanitized request id: hex only; client-supplied values are honored
    // only when they are pure hex (fixes journald control-byte injection).
    let rid = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let n = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            format!("{t:08x}{n:04x}")
        });

    let result = std::panic::AssertUnwindSafe(next.run(req))
        .catch_unwind()
        .await;
    let mut resp = match result {
        Ok(r) => r,
        Err(_) => {
            tracing::error!("handler panicked (rid={rid}) path={path}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
        }
    };
    resp.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&rid).unwrap_or(HeaderValue::from_static("rid")),
    );

    let status = resp.status().as_u16();
    if !is_streaming(&path) {
        tracing::info!(target: "access", "{method} {path} -> {status} in {:.1?} rid={rid}", start.elapsed());
    }
    resp
}

use futures_util::FutureExt;

fn is_streaming(path: &str) -> bool {
    path == "/api/events" || path == "/api/events/ws" || path.starts_with("/ui/static/")
}

/// Auth + CSRF/origin defense.
///
/// When no token is configured (open mode): mutating requests must present no
/// Origin header or a same-host one — this blocks cross-site form posts and
/// DNS-rebinding while leaving curl/HA/mqtt unaffected.
///
/// When a token is configured: everything except /api/health, /login,
/// /ui/static/* requires the token via `Authorization: Bearer`, the custom
/// `X-Auth-Token` header, the `capi_token` cookie, or `?key=`. Mutating
/// requests additionally require same-origin evidence unless the token came
/// from a custom header (custom headers cannot be sent cross-site without a
/// CORS preflight, which we never answer).
async fn auth_layer(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let token_cfg = state.auth_token();

    // Public surfaces.
    if path == "/api/health"
        || path.starts_with("/ui/static/")
        || path == "/login"
        || path == "/metrics"
    {
        return next.run(req).await;
    }

    let headers = req.headers().clone();
    let origin_ok = |origin: Option<&str>| -> bool {
        match origin {
            None => true,
            Some(o) => match url::HostParse::parse(o) {
                Some(host) => {
                    host.0
                        == host_of(
                            headers
                                .get("host")
                                .and_then(|h| h.to_str().ok())
                                .unwrap_or(""),
                        )
                }
                None => false,
            },
        }
    };

    let provided_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").map(str::to_owned))
        .or_else(|| {
            headers
                .get("x-auth-token")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        });
    let provided_cookie = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .find_map(|p| p.trim().strip_prefix("capi_token="))
                .map(str::to_owned)
        });
    let provided_query = req.uri().query().and_then(|q| {
        form_urlencoded(q)
            .into_iter()
            .find(|(k, _)| k == "key")
            .map(|(_, v)| v)
    });

    let same_origin = origin_ok(headers.get("origin").and_then(|o| o.to_str().ok()));

    if token_cfg.is_empty() {
        // Open mode: still enforce origin on mutations.
        if method == axum::http::Method::POST && !same_origin {
            return err(StatusCode::FORBIDDEN, "cross-origin request rejected");
        }
        return next.run(req).await;
    }

    let via_header = provided_header.as_deref() == Some(token_cfg.as_str());
    let via_cookie = provided_cookie.as_deref() == Some(token_cfg.as_str());
    let via_query = provided_query.as_deref() == Some(token_cfg.as_str());

    let authenticated = via_header || ((via_cookie || via_query) && same_origin);
    if !authenticated {
        // Browser UX: redirect page loads to /login; API gets 401.
        if !path.starts_with("/api") && method == axum::http::Method::GET {
            let mut r = err(StatusCode::UNAUTHORIZED, "unauthorized");
            *r.status_mut() = StatusCode::UNAUTHORIZED;
            r.headers_mut()
                .insert("location", HeaderValue::from_static("/login"));
            return r;
        }
        return err(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    if method == axum::http::Method::POST && !via_header && !same_origin {
        return err(StatusCode::FORBIDDEN, "cross-origin mutation rejected");
    }

    next.run(req).await
}

mod url {
    /// Extract the host part of an Origin header value ("https://host:port").
    pub struct HostParse(pub String);
    impl HostParse {
        pub fn parse(origin: &str) -> Option<HostParse> {
            let rest = origin.split_once("://").map(|(_, r)| r).unwrap_or(origin);
            let hostport = rest.split('/').next()?;
            if hostport.is_empty() {
                None
            } else {
                Some(HostParse(hostport.to_lowercase()))
            }
        }
    }
    impl PartialEq<str> for HostParse {
        fn eq(&self, other: &str) -> bool {
            self.0 == other
        }
    }
}

fn host_of(host_header: &str) -> String {
    host_header.trim_end_matches('/').to_lowercase()
}

fn form_urlencoded(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((percent_decode(k), percent_decode(v)))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                }
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Pages live in ui module.
        .merge(crate::ui::routes())
        // Devices / bus / topology
        .route("/api/devices", get(api::devices_handler))
        .route("/api/devices/{address}", get(api::device_handler))
        .route("/api/bus/state", get(api::bus_state_handler))
        .route("/api/bus/scan", post(api::bus_scan_handler))
        .route("/api/bus/frames", get(api::bus_frames_handler))
        .route("/api/topology", get(api::topology_handler))
        // Power
        .route("/api/power/on", post(api::power_on_handler))
        .route("/api/power/on/{address}", post(api::power_on_handler))
        .route("/api/power/off", post(api::power_off_handler))
        .route("/api/power/off/{address}", post(api::power_off_handler))
        .route("/api/power/status", get(api::power_status_handler))
        .route(
            "/api/power/status/{address}",
            get(api::power_status_handler),
        )
        // Volume
        .route("/api/volume/up", post(api::volume_up_handler))
        .route("/api/volume/up/{address}", post(api::volume_up_handler))
        .route("/api/volume/down", post(api::volume_down_handler))
        .route("/api/volume/down/{address}", post(api::volume_down_handler))
        .route("/api/volume/mute", post(api::mute_handler))
        .route("/api/volume/mute/{address}", post(api::mute_handler))
        // Source / HDMI / audio
        .route("/api/source/active", get(api::active_source_handler))
        .route("/api/source/{address}", post(api::set_source_handler))
        .route("/api/hdmi/{port}", post(api::hdmi_port_handler))
        .route("/api/audio/status", get(api::audio_status_handler))
        // Nav + raw
        .route("/api/key", post(api::send_key_handler))
        .route("/api/command", post(api::raw_command_handler))
        // Logs / streams / health / update / settings
        .route("/api/logs", get(api::logs_handler))
        .route("/api/events", get(sse::events_sse))
        .route("/api/events/ws", get(ws::events_ws))
        .route("/api/health", get(api::health_handler))
        .route("/metrics", get(metrics::metrics_handler))
        .route("/api/update", post(api::update_handler))
        .route(
            "/api/settings/mqtt",
            get(api::mqtt_settings_get).post(api::mqtt_settings_post),
        )
        // Dev surface
        .merge(dev_api::routes())
        .layer(axmw::from_fn_with_state(state.clone(), auth_layer))
        .layer(axmw::from_fn(observability))
        .with_state(state)
}
