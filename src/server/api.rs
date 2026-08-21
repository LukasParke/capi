//! JSON API handlers. Status-code discipline: malformed input is 400,
//! adapter-down is 503, upstream CEC failures are 500/502 — never string
//! sniffing (fixes the Go error-classification bugs).

use super::{accepted, err, ok, unavailable, AppState};
use crate::exec::{self, ExecError};
use crate::types::event_type;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;

fn map_exec(e: &ExecError) -> Response {
    match e {
        ExecError::AdapterUnavailable => unavailable(),
        ExecError::InvalidLogicalAddress | ExecError::InvalidHdmiPort | ExecError::InvalidKey => {
            err(StatusCode::BAD_REQUEST, e.to_string())
        }
        ExecError::Other(_) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        ExecError::Cec(_) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ---- devices / bus ---------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct DevicesQuery {
    live: Option<String>,
    rescan: Option<String>,
    wait: Option<String>,
}

const MAX_DEVICE_WAIT_SECS: u64 = 10;

pub async fn devices_handler(
    State(state): State<AppState>,
    Query(q): Query<DevicesQuery>,
) -> Response {
    if !state.0.adapter.ready() {
        return unavailable();
    }
    let wait = match &q.wait {
        Some(w) => match crate::util::parse_wait(w) {
            Ok(d) => d,
            Err(e) => return err(StatusCode::BAD_REQUEST, format!("invalid wait: {e}")),
        },
        None => std::time::Duration::ZERO,
    };
    let sync = q
        .live
        .as_deref()
        .map(crate::server::truthy)
        .unwrap_or(false)
        || q.rescan
            .as_deref()
            .map(crate::server::truthy)
            .unwrap_or(false);
    let wait = if (sync || !wait.is_zero()) && wait.is_zero() {
        std::time::Duration::from_secs(5)
    } else {
        wait.min(std::time::Duration::from_secs(MAX_DEVICE_WAIT_SECS))
    };

    let kind = if sync {
        crate::steward::JobKind::Full
    } else {
        crate::steward::JobKind::Light
    };
    let source = if sync { "live" } else { "cache" };

    if wait.is_zero() {
        state.0.steward.enqueue(kind);
        let snap = state.0.bus.copy_snapshot();
        return ok("Devices retrieved (cache)", Some(json!(snap.devices)));
    }

    match state.0.steward.enqueue_wait(kind, wait).await {
        Ok(()) => {
            let snap = state.0.bus.copy_snapshot();
            ok(
                format!("Devices retrieved ({source})"),
                Some(json!(snap.devices)),
            )
        }
        Err(crate::steward::StewardWait::QueueFull) => err(
            StatusCode::SERVICE_UNAVAILABLE,
            "Bus steward queue full; retry shortly",
        ),
        Err(crate::steward::StewardWait::Timeout) => err(
            StatusCode::GATEWAY_TIMEOUT,
            "Device scan timed out; partial result via /api/bus/state",
        ),
    }
}

pub async fn device_handler(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Response {
    let addr = match address.parse::<i32>() {
        Ok(a) if (0..=14).contains(&a) => a,
        _ => return err(StatusCode::BAD_REQUEST, "invalid logical address"),
    };
    let Some(conn) = state.0.adapter.get() else {
        return unavailable();
    };
    let res = tokio::task::spawn_blocking(move || {
        conn.get_device_info(crate::cec::LogicalAddress(addr as u8))
    })
    .await;
    match res {
        Ok(Ok(info)) => ok("Device retrieved", Some(json!(info.to_map()))),
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

pub async fn bus_state_handler(State(state): State<AppState>) -> Response {
    let snap = state.0.bus.copy_snapshot();
    ok(
        "Bus state",
        Some(json!({
            "devices": snap.devices,
            "logical_addresses": snap.logical_addresses,
            "active_source": snap.active_source,
            "cec_ready": snap.cec_ready,
            "monitoring": snap.monitoring,
            "scan_in_progress": snap.scan_in_progress,
            "stale": snap.stale,
            "last_full_scan_at": snap.last_full_scan_at.map(|t| t.to_rfc3339()),
            "stale_threshold_sec": snap.stale_threshold_sec,
            "generation": snap.generation,
        })),
    )
}

pub async fn bus_scan_handler(State(state): State<AppState>) -> Response {
    let enqueued = state.0.steward.enqueue(crate::steward::JobKind::Deep);
    if enqueued {
        accepted("Scan queued", Some(json!({"accepted": true})))
    } else {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "Bus steward queue full; retry shortly",
        )
    }
}

pub async fn topology_handler(State(state): State<AppState>) -> Response {
    // Served from the steward snapshot — never blocks on the serial bus.
    let topo = crate::topology::build_from_snapshot(&state.0.bus);
    ok("Topology", Some(json!(topo)))
}

pub async fn bus_frames_handler(State(state): State<AppState>) -> Response {
    let frames = state.0.bus.recent_frames();
    ok("Frames", Some(json!(frames)))
}

// ---- power -----------------------------------------------------------------

pub async fn power_on_handler(
    State(state): State<AppState>,
    path: Option<Path<String>>,
) -> Response {
    let addr = optional_addr(path, 0);
    match exec::power_on(&state.0.adapter, &state.0.steward, addr) {
        Ok(()) => ok(format!("Power on command sent to device {addr}"), None),
        Err(e) => map_exec(&e),
    }
}

pub async fn power_off_handler(
    State(state): State<AppState>,
    path: Option<Path<String>>,
) -> Response {
    let addr = optional_addr(path, 0);
    match exec::power_off(&state.0.adapter, &state.0.steward, addr).err() {
        None => ok(format!("Standby command sent to device {addr}"), None),
        Some(e) => map_exec(&e),
    }
}

pub async fn power_status_handler(
    State(state): State<AppState>,
    path: Option<Path<String>>,
) -> Response {
    let addr = optional_addr(path, 0);
    match exec::power_status(&state.0.adapter, addr) {
        Ok(status) => ok(
            "Power status retrieved",
            Some(json!({"address": addr, "status": status})),
        ),
        Err(e) => map_exec(&e),
    }
}

fn optional_addr(path: Option<Path<String>>, default: i32) -> i32 {
    path.and_then(|p| p.0.parse::<i32>().ok())
        .unwrap_or(default)
}

// ---- volume ----------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct VolumeQuery {
    address: Option<String>,
}

async fn volume(state: AppState, action: crate::strategies::Action, q: VolumeQuery) -> Response {
    let addr = match &q.address {
        None => None,
        Some(s) if s.trim().is_empty() => None,
        Some(s) => match s.parse::<i32>() {
            Ok(a) if (0..=14).contains(&a) => Some(a),
            _ => return err(StatusCode::BAD_REQUEST, "invalid logical address"),
        },
    };
    let Some(conn) = state.0.adapter.get() else {
        return unavailable();
    };
    let bus = state.0.bus.clone();
    let registry = state.0.registry.clone();
    let res = tokio::task::spawn_blocking(move || {
        exec::volume_action(&conn, &bus, &registry, action, addr)
    })
    .await;
    match res {
        Ok(Ok(msg)) => ok(msg, None),
        Ok(Err(e)) => map_exec(&e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

pub async fn volume_up_handler(
    State(state): State<AppState>,
    Query(q): Query<VolumeQuery>,
) -> Response {
    volume(state, crate::strategies::Action::VolumeUp, q).await
}
pub async fn volume_down_handler(
    State(state): State<AppState>,
    Query(q): Query<VolumeQuery>,
) -> Response {
    volume(state, crate::strategies::Action::VolumeDown, q).await
}
pub async fn mute_handler(State(state): State<AppState>, Query(q): Query<VolumeQuery>) -> Response {
    volume(state, crate::strategies::Action::Mute, q).await
}

// ---- source / hdmi / audio ---------------------------------------------------

pub async fn active_source_handler(State(state): State<AppState>) -> Response {
    let Some(conn) = state.0.adapter.get() else {
        return unavailable();
    };
    let res = tokio::task::spawn_blocking(move || conn.get_active_source()).await;
    match res {
        Ok(Ok(src)) => ok(
            "Active source retrieved",
            Some(json!({"active_source": src.0})),
        ),
        Ok(Err(crate::cec::CecError::NoActiveSource)) => {
            ok("No active source", Some(json!({"active_source": -1})))
        }
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

pub async fn set_source_handler(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Response {
    let addr = match address.parse::<i32>() {
        Ok(a) if (0..=14).contains(&a) => a,
        _ => return err(StatusCode::BAD_REQUEST, "invalid logical address"),
    };
    match exec::set_active_source(&state.0.adapter, &state.0.steward, addr).err() {
        None => ok(format!("Source switched to device {addr}"), None),
        Some(e) => map_exec(&e),
    }
}

pub async fn hdmi_port_handler(
    State(state): State<AppState>,
    Path(port): Path<String>,
) -> Response {
    let port = match port.parse::<i32>() {
        Ok(p) if (1..=15).contains(&p) => p,
        _ => return err(StatusCode::BAD_REQUEST, "invalid HDMI port"),
    };
    match exec::hdmi_port(&state.0.adapter, &state.0.steward, port).err() {
        None => ok(format!("Switched to HDMI port {port}"), None),
        Some(e) => map_exec(&e),
    }
}

pub async fn audio_status_handler(State(state): State<AppState>) -> Response {
    let (volume, muted, raw) = exec::audio_status(&state.0.adapter);
    ok(
        "Audio status",
        Some(json!({"volume": volume, "muted": muted, "raw": raw})),
    )
}

// ---- key / raw command -------------------------------------------------------

#[derive(Deserialize)]
pub struct KeyRequest {
    #[serde(default)]
    address: i32,
    #[serde(default)]
    key: String,
    #[serde(default)]
    keycode: i32,
}

pub async fn send_key_handler(
    State(state): State<AppState>,
    body: Option<axum::Json<KeyRequest>>,
) -> Response {
    let Some(axum::Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, "invalid request body");
    };
    if !(0..=14).contains(&req.address) {
        return err(StatusCode::BAD_REQUEST, "invalid logical address");
    }
    if req.key.is_empty() && req.keycode == 0 {
        return err(
            StatusCode::BAD_REQUEST,
            "either 'key' or 'keycode' must be provided (keycode 0 = select; use key:\"select\")",
        );
    }
    let AppState(inner) = state.clone();
    let conn = inner.adapter.get();
    let Some(conn) = conn else {
        return unavailable();
    };
    let bus = inner.bus.clone();
    let registry = inner.registry.clone();
    let res = tokio::task::spawn_blocking(move || {
        exec::send_key(
            &inner.adapter,
            &bus,
            &registry,
            req.address,
            &req.key,
            req.keycode,
        )
    })
    .await;
    // keep conn alive for the duration of the call
    drop(conn);
    match res {
        Ok(Ok(msg)) => ok(msg, None),
        Ok(Err(e)) => map_exec(&e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

#[derive(Deserialize)]
pub struct RawCommandRequest {
    initiator: i32,
    destination: i32,
    opcode: i32,
    #[serde(default)]
    parameters: Vec<u8>,
}

pub async fn raw_command_handler(
    State(state): State<AppState>,
    body: Option<axum::Json<RawCommandRequest>>,
) -> Response {
    let Some(axum::Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, "invalid request body");
    };
    if !(0..=15).contains(&req.initiator) {
        return err(
            StatusCode::BAD_REQUEST,
            "invalid initiator logical address (must be 0-15)",
        );
    }
    if !(0..=15).contains(&req.destination) {
        return err(
            StatusCode::BAD_REQUEST,
            "invalid destination logical address (must be 0-15)",
        );
    }
    if !(0..=255).contains(&req.opcode) {
        return err(StatusCode::BAD_REQUEST, "invalid opcode (must be 0-255)");
    }
    const MAX_CEC_PARAMETERS: usize = 14;
    if req.parameters.len() > MAX_CEC_PARAMETERS {
        return err(
            StatusCode::BAD_REQUEST,
            format!("too many parameters (max {MAX_CEC_PARAMETERS})"),
        );
    }
    let Some(conn) = state.0.adapter.get() else {
        return unavailable();
    };
    let cmd = crate::cec::Command {
        initiator: crate::cec::LogicalAddress(req.initiator as u8),
        destination: crate::cec::LogicalAddress(req.destination as u8),
        opcode: crate::cec::Opcode(req.opcode as u8),
        opcode_set: true,
        parameters: req.parameters,
        ack: false,
        eom: true,
    };
    let res = tokio::task::spawn_blocking(move || conn.transmit(&cmd)).await;
    match res {
        Ok(Ok(())) => {
            state.0.steward.enqueue(crate::steward::JobKind::Light);
            ok("Raw command sent", None)
        }
        Ok(Err(crate::cec::CecError::AdapterNotOpen)) => unavailable(),
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

// ---- logs / health -----------------------------------------------------------

pub async fn logs_handler(State(state): State<AppState>) -> Response {
    ok("Logs retrieved", Some(json!(state.0.logs.recent())))
}

pub async fn health_handler(State(state): State<AppState>) -> Response {
    let hub = &state.0.hub;
    let (dropped, delivered) = hub.stats();
    let uptime = chrono::Utc::now() - state.0.started;
    ok(
        "healthy",
        Some(json!({
            "version": env!("CAPI_VERSION"),
            "uptime_seconds": uptime.num_seconds(),
            "cec_ready": state.0.adapter.ready(),
            "subscribers": hub.subscriber_count(),
            "events_dropped": dropped,
            "events_delivered": delivered,
            "frames_captured": state.0.bus.frames_captured.load(std::sync::atomic::Ordering::Relaxed),
        })),
    )
}

// ---- update ------------------------------------------------------------------

pub async fn update_handler(State(state): State<AppState>) -> Response {
    match crate::update::check_and_perform(&state.0.settings).await {
        Ok(Some(newver)) => ok(
            format!("Updated to {newver}, restarting..."),
            Some(json!({"new_version": newver})),
        ),
        Ok(None) => ok(
            "Already up to date",
            Some(json!({"version": env!("CAPI_VERSION")})),
        ),
        Err(e) => err(StatusCode::BAD_GATEWAY, format!("update failed: {e}")),
    }
}

// ---- MQTT settings ------------------------------------------------------------

pub async fn mqtt_settings_get(State(state): State<AppState>) -> Response {
    let cfg = state.0.settings.get().mqtt;
    let connected = state.mqtt_connected();
    ok(
        "MQTT settings",
        Some(json!({
            "broker": cfg.broker,
            "user": cfg.user,
            "pass": if cfg.pass.is_empty() { "" } else { "***" },
            "prefix": cfg.prefix,
            "connected": connected,
        })),
    )
}

#[derive(Deserialize)]
pub struct MqttSettingsRequest {
    #[serde(default)]
    broker: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    pass: String,
    #[serde(default)]
    prefix: String,
}

pub async fn mqtt_settings_post(
    State(state): State<AppState>,
    body: Option<axum::Json<MqttSettingsRequest>>,
) -> Response {
    let Some(axum::Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, "invalid request body");
    };
    let existing = state.0.settings.get().mqtt;
    let pass = match req.pass.as_str() {
        "***" => existing.pass.clone(),
        "" => String::new(),
        other => other.to_string(),
    };
    let cfg = crate::types::MqttConfig {
        broker: req.broker.trim().to_string(),
        user: req.user.trim().to_string(),
        pass,
        prefix: if req.prefix.trim().is_empty() {
            "capi".into()
        } else {
            req.prefix.trim().to_string()
        },
    };
    if let Err(e) = state.0.settings.update(|c| c.mqtt = cfg.clone()) {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("save config: {e}"),
        );
    }
    state.apply_mqtt_config(&cfg);
    ok("MQTT settings saved", None)
}

// re-exported for query truthiness

impl AppState {
    #[allow(dead_code)]
    pub fn publish_event(&self, kind: &str, data: serde_json::Value) {
        self.0.hub.publish(crate::types::AppEvent::new(kind, data));
    }

    #[allow(dead_code)]
    pub fn note_event(&self, kind: &str, data: serde_json::Value) {
        self.publish_event(kind, data);
        self.0
            .metrics
            .events_published
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub fn event_type_names() -> [&'static str; 8] {
        [
            event_type::POWER_CHANGE,
            event_type::SOURCE_ACTIVATED,
            event_type::KEY_PRESS,
            event_type::COMMAND,
            event_type::ALERT,
            event_type::DEVICES_CHANGED,
            event_type::CONFIGURATION_CHANGED,
            event_type::ADAPTER_STATE,
        ]
    }
}
