//! Dev/unstable surface: probes, strategy bench, raw sends, mode toggles.
//! All client-tunable timings clamped (fix: unbounded request-driven sleeps).

use super::{err, ok, unavailable, AppState};
use crate::cec::LogicalAddress;
use crate::strategies::{RunOptions, MAX_HOLD_MS, MAX_OBSERVE_MS};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/dev/mode", get(get_mode).post(post_mode))
        .route("/api/dev/probe", post(probe))
        .route("/api/dev/send_key", post(send_key))
        .route("/api/dev/send_opcode", post(send_opcode))
        .route("/api/dev/run_strategies", post(run_strategies))
        .route("/api/dev/save_strategy", post(save_strategy))
        .route("/api/dev/actions", get(actions))
        .route("/api/dev/keys", get(keys))
        .route("/api/dev/opcodes", get(opcodes))
}

#[derive(Deserialize)]
pub struct ModeBody {
    monitor_only: Option<bool>,
    #[serde(default)]
    reconnect: bool,
}

async fn get_mode(State(state): State<AppState>) -> Response {
    let cfg = state.0.settings.get();
    ok("mode", Some(json!({"monitor_only": cfg.cec.monitor_only})))
}

async fn post_mode(State(state): State<AppState>, body: Option<Json<ModeBody>>) -> Response {
    let Some(Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, "invalid body");
    };
    if req.reconnect {
        state.0.adapter.signal_reconnect();
        return ok("reconnect requested", None);
    }
    let Some(monitor) = req.monitor_only else {
        return err(
            StatusCode::BAD_REQUEST,
            "monitor_only or reconnect required",
        );
    };
    // Update-and-persist atomically; memory never diverges from disk.
    if let Err(e) = state.0.settings.update(|c| c.cec.monitor_only = monitor) {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("save config: {e}"),
        );
    }
    state.0.adapter.signal_reconnect();
    ok(
        format!(
            "mode set to {} — reconnecting",
            if monitor { "monitor" } else { "passive" }
        ),
        None,
    )
}

#[derive(Deserialize)]
pub struct ProbeRequest {
    address: i32,
    #[serde(default)]
    kind: String,
    observe_ms: Option<i64>,
}

const PROBE_KINDS: &[(&str, crate::cec::Opcode)] = &[
    ("power", crate::cec::Opcode::GIVE_DEVICE_POWER_STATUS),
    ("vendor", crate::cec::Opcode::GIVE_DEVICE_VENDOR_ID),
    ("osd", crate::cec::Opcode::GIVE_OSD_NAME),
    ("cec_version", crate::cec::Opcode::GET_CEC_VERSION),
    ("physical", crate::cec::Opcode::GIVE_PHYSICAL_ADDRESS),
];

async fn probe(State(state): State<AppState>, body: Option<Json<ProbeRequest>>) -> Response {
    let Some(Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, "invalid JSON body");
    };
    if !(0..=14).contains(&req.address) {
        return err(StatusCode::BAD_REQUEST, "address must be 0..14");
    }
    let Some(conn) = state.0.adapter.get() else {
        return unavailable();
    };
    if conn.is_monitor_only() {
        return err(
            StatusCode::CONFLICT,
            "adapter is in monitor-only mode; switch to passive first",
        );
    }
    let observe_ms = req.observe_ms.unwrap_or(600).clamp(100, MAX_OBSERVE_MS);
    let kind = if req.kind.is_empty() {
        "all".to_string()
    } else {
        req.kind.to_lowercase()
    };

    let wanted: Vec<&(&str, crate::cec::Opcode)> = if kind == "all" {
        PROBE_KINDS.iter().collect()
    } else {
        PROBE_KINDS
            .iter()
            .filter(|(n, _)| n.starts_with(&kind))
            .collect()
    };
    if wanted.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            format!("unknown probe kind {:?}", req.kind),
        );
    }

    let bus = state.0.bus.clone();
    let ring_cap = state.0.bus.frame_ring_capacity();
    let res = tokio::task::spawn_blocking(move || {
        let mut steps = Vec::new();
        let mut total_replies = 0usize;
        for (name, op) in wanted {
            let pre_seq = bus.ring_high_water();
            let start = std::time::Instant::now();
            let send_err = conn
                .transmit(&crate::cec::Command {
                    initiator: conn.first_logical_address().unwrap_or(LogicalAddress::FREE_USE),
                    destination: LogicalAddress(req.address as u8),
                    opcode: *op,
                    opcode_set: true,
                    parameters: Vec::new(),
                    ack: false,
                    eom: true,
                })
                .err();
            std::thread::sleep(std::time::Duration::from_millis(observe_ms as u64));
            let replies: Vec<serde_json::Value> =
                bus.frames_after(pre_seq).iter().map(|f| json!({ "opcode": f.opcode, "params": f.params_hex })).collect();
            total_replies += replies.len();
            steps.push(json!({
                "name": name,
                "opcode": format!("0x{:02X}", op.0),
                "result": if send_err.is_some() { "error" } else if replies.is_empty() { "no_reply" } else { "ok" },
                "error": send_err.map(|e| format!("{e:#}")).unwrap_or_default(),
                "elapsed_ms": start.elapsed().as_millis() as i64,
                "replies": replies,
            }));
        }
        (steps, total_replies, ring_cap)
    })
    .await;

    match res {
        Ok((steps, total_replies, _)) => ok(
            "probe complete",
            Some(json!({
                "address": req.address,
                "kind": kind,
                "observe_ms": observe_ms,
                "total_replies": total_replies,
                "steps": steps,
            })),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

#[derive(Deserialize)]
pub struct SendKeyRequest {
    address: i32,
    #[serde(default)]
    key: String,
    keycode: Option<i32>,
    hold_ms: Option<i64>,
    repeat: Option<i32>,
}

async fn send_key(State(state): State<AppState>, body: Option<Json<SendKeyRequest>>) -> Response {
    let Some(Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, "invalid JSON body");
    };
    let AppState(inner) = state.clone();
    let Some(conn) = inner.adapter.get() else {
        return unavailable();
    };
    let hold = req.hold_ms.unwrap_or(0).clamp(0, MAX_HOLD_MS);
    let repeat = req.repeat.unwrap_or(1).clamp(1, 32);
    let keycode = req.keycode.unwrap_or(0);
    let res = tokio::task::spawn_blocking(move || {
        let mut results = Vec::new();
        for _ in 0..repeat {
            match crate::exec::send_key(
                &inner.adapter,
                &inner.bus,
                &inner.registry,
                req.address,
                &req.key,
                keycode,
            ) {
                Ok(msg) => results.push(msg),
                Err(e) => return Err(e),
            }
            if hold > 0 {
                std::thread::sleep(std::time::Duration::from_millis(hold as u64));
            }
        }
        drop(conn);
        Ok(results.join("; "))
    })
    .await;
    match res {
        Ok(Ok(msg)) => ok(msg, None),
        Ok(Err(e)) => super::api_map_exec(&e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

#[derive(Deserialize)]
pub struct SendOpcodeRequest {
    destination: i32,
    opcode: i32,
    #[serde(default)]
    params_hex: String,
    observe_ms: Option<i64>,
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let t = t.strip_prefix("0x").unwrap_or(&t);
    if !t.len().is_multiple_of(2) {
        return Err("odd hex length".into());
    }
    (0..t.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&t[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

async fn send_opcode(
    State(state): State<AppState>,
    body: Option<Json<SendOpcodeRequest>>,
) -> Response {
    let Some(Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, "invalid JSON body");
    };
    if !(0..=15).contains(&req.destination) {
        return err(StatusCode::BAD_REQUEST, "destination must be 0..15");
    }
    if !(0..=255).contains(&req.opcode) {
        return err(StatusCode::BAD_REQUEST, "opcode must be 0..255");
    }
    let params = match parse_hex_bytes(&req.params_hex) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::BAD_REQUEST, format!("invalid params_hex: {e}")),
    };
    if params.len() > 14 {
        return err(StatusCode::BAD_REQUEST, "too many parameters (max 14)");
    }
    let Some(conn) = state.0.adapter.get() else {
        return unavailable();
    };
    if conn.is_monitor_only() {
        return err(StatusCode::CONFLICT, "adapter is in monitor-only mode");
    }
    let observe = req.observe_ms.unwrap_or(700).clamp(100, MAX_OBSERVE_MS);
    let bus = state.0.bus.clone();
    let res = tokio::task::spawn_blocking(move || {
        let pre_seq = bus.ring_high_water();
        let tx_err = conn
            .transmit(&crate::cec::Command {
                initiator: conn
                    .first_logical_address()
                    .unwrap_or(LogicalAddress::FREE_USE),
                destination: LogicalAddress(req.destination as u8),
                opcode: crate::cec::Opcode(req.opcode as u8),
                opcode_set: true,
                parameters: params,
                ack: false,
                eom: true,
            })
            .err();
        std::thread::sleep(std::time::Duration::from_millis(observe as u64));
        let new_frames = bus.frames_after(pre_seq);
        (tx_err, new_frames)
    })
    .await;
    match res {
        Ok((tx_err, frames)) => ok(
            "sent",
            Some(json!({
                "transmit_error": tx_err.map(|e| format!("{e:#}")),
                "new_frames": frames,
            })),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

#[derive(Deserialize)]
pub struct RunStrategiesRequest {
    action: String,
    target: Option<i32>,
    observe_ms: Option<i64>,
    all_strategies: Option<bool>,
}

async fn run_strategies(
    State(state): State<AppState>,
    body: Option<Json<RunStrategiesRequest>>,
) -> Response {
    let Some(Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, "invalid JSON body");
    };
    let Some(action) = crate::strategies::Action::parse(&req.action) else {
        return err(
            StatusCode::BAD_REQUEST,
            format!("unknown action {:?}", req.action),
        );
    };
    let target = match req.target {
        None | Some(-1) => None,
        Some(t) if (0..=14).contains(&t) => Some(LogicalAddress(t as u8)),
        Some(t) => return err(StatusCode::BAD_REQUEST, format!("invalid target {t}")),
    };
    let Some(conn) = state.0.adapter.get() else {
        return unavailable();
    };
    if conn.is_monitor_only() {
        return err(StatusCode::CONFLICT, "adapter is in monitor-only mode");
    }
    let bus = state.0.bus.clone();
    let registry = state.0.registry.clone();
    let vendor = target
        .map(|t| crate::exec::vendor_id_for_target(&bus, t))
        .unwrap_or_default();
    let opts = RunOptions {
        vendor,
        target,
        all_strategies: req.all_strategies.unwrap_or(true),
        observe_override_ms: req.observe_ms.unwrap_or(0).clamp(0, MAX_OBSERVE_MS),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let res =
        tokio::task::spawn_blocking(move || registry.run(&conn, &bus, action, &opts, deadline))
            .await;
    match res {
        Ok(results) => ok(
            "run complete",
            Some(json!({ "action": action.as_str(), "results": results })),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

#[derive(Deserialize)]
pub struct SaveStrategyRequest {
    vendor: String,
    action: String,
    strategy: String,
}

async fn save_strategy(
    State(state): State<AppState>,
    body: Option<Json<SaveStrategyRequest>>,
) -> Response {
    let Some(Json(req)) = body else {
        return err(StatusCode::BAD_REQUEST, "invalid JSON body");
    };
    let Some(action) = crate::strategies::Action::parse(&req.action) else {
        return err(
            StatusCode::BAD_REQUEST,
            format!("unknown action {:?}", req.action),
        );
    };
    let chain = state.0.registry.strategies_for("", action);
    let Some(picked) = chain.iter().find(|s| s.name == req.strategy).cloned() else {
        return err(
            StatusCode::NOT_FOUND,
            format!("strategy {:?} not found for {}", req.strategy, req.action),
        );
    };
    state
        .0
        .registry
        .set_vendor_override(&req.vendor, action, vec![picked]);
    let persist = state.0.settings.update(|c| {
        c.cec
            .strategy_overrides
            .entry(req.vendor.clone())
            .or_default()
            .insert(req.action.clone(), req.strategy.clone());
    });
    match persist {
        Ok(()) => ok("override saved and applied", None),
        Err(e) => err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("save config: {e}"),
        ),
    }
}

async fn actions(State(state): State<AppState>) -> Response {
    let list: Vec<serde_json::Value> = crate::strategies::ALL_ACTIONS
        .iter()
        .map(|(name, a)| {
            let chain = state.0.registry.default_chain(*a);
            json!({ "action": name, "strategies": chain.iter().map(|(n, _)| n).collect::<Vec<_>>() })
        })
        .collect();
    ok("actions", Some(json!(list)))
}

async fn keys() -> Response {
    ok("keys", Some(json!(crate::cec::keycode_names())))
}

async fn opcodes() -> Response {
    ok("opcodes", Some(json!(crate::cec::opcode_table())))
}
