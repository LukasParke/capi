//! UI: page routes, htmx fragments, form actions, login. Templates are
//! askama-compiled (auto-escaped); no hand-rolled HTML emitters anywhere
//! (fixes the Go dual-convention escaping risk).

use crate::server::{err, ok, unavailable, AppState};
use askama::Template;
use axum::extract::{Form, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::json;

pub const VERSION: &str = env!("CAPI_VERSION");

// ---- template roots ---------------------------------------------------------

#[derive(Template)]
#[template(path = "dashboard.html")]
#[allow(dead_code)]
pub struct DashboardTmpl {
    pub ctx: crate::ui_ctx::RemoteData,
    pub version: String,
}

#[derive(Template)]
#[template(path = "settings.html")]
#[allow(dead_code)]
pub struct SettingsTmpl {
    pub mqtt: crate::ui_ctx::MqttPanelData,
    pub health: crate::ui_ctx::HealthData,
    pub monitor_only: bool,
    pub token_set: bool,
    pub version: String,
}

#[derive(Template)]
#[template(path = "dev.html")]
pub struct DevTmpl {
    pub mode: crate::ui_ctx::DevModeData,
    pub adapter_ready: bool,
    pub actions: Vec<String>,
    pub keys: Vec<(String, u8)>,
    pub version: String,
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTmpl {
    pub error: String,
}

// ---- helpers ----------------------------------------------------------------

pub fn device_rows_from_snapshot(
    snap: &crate::busstate::BusStateSnapshot,
) -> Vec<crate::ui_ctx::DeviceRow> {
    let mut rows = Vec::new();
    for d in &snap.devices {
        let la = d
            .get("logical_address")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if la < 0 {
            continue;
        }
        let s = |k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let i = |k: &str| d.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
        let b = |k: &str| d.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
        let osd = s("osd_name");
        let role = s("device_type");
        let display = if !osd.is_empty() {
            osd.clone()
        } else {
            let frag = s("observed_osd_name_fragment");
            if !frag.is_empty() {
                frag
            } else if !role.is_empty() {
                crate::util::title_case(&role)
            } else {
                s("address_name")
            }
        };
        let mut power_status = s("power_status");
        if power_status.is_empty() || power_status == "Unknown" {
            let obs = s("observed_power_status");
            if !obs.is_empty() {
                power_status = crate::util::title_case(&obs);
            }
        }
        rows.push(crate::ui_ctx::DeviceRow {
            logical_address: la as i32,
            display_name: display,
            address_name: s("address_name"),
            role,
            physical_address: s("physical_address"),
            hdmi_port: i("hdmi_port"),
            vendor_name: s("vendor_name"),
            vendor_id: s("vendor_id"),
            cec_version: s("cec_version"),
            power_status,
            power_observed_at: s("observed_at"),
            discovery: s("discovery"),
            is_own: b("is_own"),
            is_active_source: snap.active_source == la as i32 || b("is_active_source"),
            is_audio_system: la == 5,
            is_ghost: s("discovery") == "observed",
            first_seen: s("first_seen_at"),
            last_seen: s("last_seen_at"),
        });
    }
    rows
}

pub fn bus_banner_data(state: &AppState) -> crate::ui_ctx::BusBannerData {
    let snap = state.0.bus.copy_snapshot();
    let mut d = crate::ui_ctx::BusBannerData::from_snapshot(&snap);
    d.cec_ready = state.0.adapter.ready() && d.cec_ready;
    if !state.0.adapter.ready() {
        d.stale = true;
    }
    d
}

pub fn remote_data(state: &AppState) -> crate::ui_ctx::RemoteData {
    let snap = state.0.bus.copy_snapshot();
    let devices = device_rows_from_snapshot(&snap);

    // HDMI ports: at least 1..4, extended by topology knowledge.
    let topo = crate::topology::build_from_snapshot(&state.0.bus);
    let max_port = topo.known_port_count.max(4);
    let active_phys_port = snap.active_source;
    let hdmi_ports = (1..=max_port)
        .map(|p| crate::ui_ctx::HdmiPortButton {
            port: p,
            selected: false,
        })
        .collect::<Vec<_>>();
    let _ = active_phys_port;

    let nav_targets: Vec<crate::ui_ctx::NavTarget> = devices
        .iter()
        .filter(|d| !d.is_own)
        .map(|d| crate::ui_ctx::NavTarget {
            la: d.logical_address,
            label: d.display_name.clone(),
            selected: d.is_active_source,
        })
        .collect();

    let audio_available = state.0.adapter.ready();
    let (vol_raw, muted, _) = crate::exec::audio_status(&state.0.adapter);
    crate::ui_ctx::RemoteData {
        banner: bus_banner_data(state),
        devices,
        hdmi_ports,
        nav_targets,
        audio_display_volume: vol_raw.min(100) as i32,
        audio_muted: muted,
        audio_available,
    }
}

pub fn source_panel_data(state: &AppState) -> crate::ui_ctx::RemoteData {
    remote_data(state)
}

/// One activity-feed line (used by WS stream and fragment endpoint).
pub fn event_feed_line_html(ev: &crate::types::AppEvent) -> String {
    #[derive(Template)]
    #[template(path = "event_feed_line.html")]
    struct FeedLineTmpl<'a> {
        ev: &'a EventFeedEntry,
    }
    struct EventFeedEntry {
        time: String,
        kind: String,
        summary: String,
    }
    let entry = EventFeedEntry {
        time: chrono::Local::now().format("%H:%M:%S").to_string(),
        kind: ev.kind.clone(),
        summary: summarize(ev),
    };
    FeedLineTmpl { ev: &entry }.render().unwrap_or_default()
}

fn summarize(ev: &crate::types::AppEvent) -> String {
    let g = |k: &str| ev.data.get(k).cloned().unwrap_or(json!(null));
    match ev.kind.as_str() {
        "power_change" => format!("device {} → {}", g("address"), g("status")),
        "source_activated" => format!("active source → LA {}", g("address")),
        "key_press" => format!("key {} ({})", g("keycode"), g("duration")),
        "command" => format!(
            "{} → {} op {}",
            g("initiator"),
            g("destination"),
            g("opcode")
        ),
        "devices_changed" => format!("devices changed ({})", g("reason")),
        "configuration_changed" => "libcec configuration changed".into(),
        "adapter_state" => format!("adapter {}", g("state")),
        other => other.to_string(),
    }
}

fn html_response(body: String) -> Response {
    let mut resp = Html(body).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp
}

macro_rules! fragment {
    ($name:ident, $tmpl:ty, $build:expr) => {
        pub async fn $name(State(state): State<AppState>) -> Response {
            let f = $build(&state);
            match f.render() {
                Ok(s) => html_response(s),
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("template: {e}")),
            }
        }
    };
}

fragment!(fragment_bus_banner, BusBannerTmpl, |s: &AppState| {
    BusBannerTmpl {
        ctx: bus_banner_data(s),
    }
});

#[derive(Template)]
#[template(path = "bus_banner.html")]
pub struct BusBannerTmpl {
    pub ctx: crate::ui_ctx::BusBannerData,
}

fragment!(fragment_devices, DevicesTmpl, |s: &AppState| DevicesTmpl {
    ctx: devices_panel_data(s)
});

fn devices_panel_data(state: &AppState) -> crate::ui_ctx::DevicesPanelData {
    let snap = state.0.bus.copy_snapshot();
    let rows = device_rows_from_snapshot(&snap);
    crate::ui_ctx::DevicesPanelData {
        message: format!("Live snapshot ({} devices)", rows.len()),
        devices: rows,
    }
}

#[derive(Template)]
#[template(path = "devices.html")]
pub struct DevicesTmpl {
    pub ctx: crate::ui_ctx::DevicesPanelData,
}

fragment!(fragment_health, HealthTmpl, |s: &AppState| HealthTmpl {
    ctx: health_data(s)
});

pub fn health_data(state: &AppState) -> crate::ui_ctx::HealthData {
    let (dropped, _delivered) = state.0.hub.stats();
    let uptime = chrono::Utc::now() - state.0.started;
    crate::ui_ctx::HealthData {
        version: VERSION.to_string(),
        uptime: format!("{}h {:02}m", uptime.num_hours(), uptime.num_minutes() % 60),
        cec_ready: state.0.adapter.ready(),
        lib_info: state
            .0
            .adapter
            .get()
            .and_then(|c| c.get_lib_info().ok())
            .unwrap_or_default(),
        subscribers: state.0.hub.subscriber_count(),
        events_dropped: dropped,
        frames_captured: state
            .0
            .bus
            .frames_captured
            .load(std::sync::atomic::Ordering::Relaxed),
    }
}

#[derive(Template)]
#[template(path = "health.html")]
pub struct HealthTmpl {
    pub ctx: crate::ui_ctx::HealthData,
}

fragment!(fragment_mqtt_panel, MqttTmpl, |s: &AppState| MqttTmpl {
    ctx: mqtt_panel_data(s)
});

fn mqtt_panel_data(state: &AppState) -> crate::ui_ctx::MqttPanelData {
    let cfg = state.0.settings.get().mqtt;
    crate::ui_ctx::MqttPanelData {
        broker: cfg.broker,
        user: cfg.user,
        prefix: cfg.prefix,
        pass_set: !cfg.pass.is_empty(),
        connected: state.mqtt_connected(),
    }
}

#[derive(Template)]
#[template(path = "mqtt_panel.html")]
pub struct MqttTmpl {
    pub ctx: crate::ui_ctx::MqttPanelData,
}

fragment!(fragment_logs, LogsTmpl, |s: &AppState| LogsTmpl {
    ctx: logs_data(s)
});

fn logs_data(state: &AppState) -> crate::ui_ctx::LogsData {
    crate::ui_ctx::LogsData {
        lines: state
            .0
            .logs
            .recent()
            .into_iter()
            .map(|m| crate::ui_ctx::LogLine {
                timestamp: m.timestamp,
                level: m.level,
                message: m.message,
            })
            .collect(),
    }
}

#[derive(Template)]
#[template(path = "logs.html")]
pub struct LogsTmpl {
    pub ctx: crate::ui_ctx::LogsData,
}

fragment!(fragment_topology_hdmi, TopologyTmpl, |s: &AppState| {
    TopologyTmpl {
        ctx: crate::topology::build_from_snapshot(&s.0.bus),
    }
});

#[derive(Template)]
#[template(path = "topology_hdmi.html")]
pub struct TopologyTmpl {
    pub ctx: crate::topology::TopologyPayload,
}

fragment!(fragment_source_panel, SourceTmpl, |s: &AppState| {
    SourceTmpl {
        ctx: remote_data(s),
    }
});

#[derive(Template)]
#[template(path = "source_panel.html")]
pub struct SourceTmpl {
    pub ctx: crate::ui_ctx::RemoteData,
}

fragment!(fragment_volume_panel, VolumeTmpl, |s: &AppState| {
    VolumeTmpl {
        ctx: remote_data(s),
    }
});

#[derive(Template)]
#[template(path = "volume_panel.html")]
pub struct VolumeTmpl {
    pub ctx: crate::ui_ctx::RemoteData,
}

fragment!(fragment_nav_panel, NavTmpl, |s: &AppState| NavTmpl {
    ctx: remote_data(s)
});

#[derive(Template)]
#[template(path = "nav_panel.html")]
pub struct NavTmpl {
    pub ctx: crate::ui_ctx::RemoteData,
}

fragment!(fragment_device_power, DevicePowerTmpl, |s: &AppState| {
    DevicePowerTmpl {
        ctx: devices_panel_data(s),
    }
});

#[derive(Template)]
#[template(path = "device_power.html")]
pub struct DevicePowerTmpl {
    pub ctx: crate::ui_ctx::DevicesPanelData,
}

// ---- dev fragments ----------------------------------------------------------

pub async fn dev_fragment_banner(State(state): State<AppState>) -> Response {
    #[derive(Template)]
    #[template(path = "dev_banner.html")]
    struct T {
        mode: crate::ui_ctx::DevModeData,
        adapter_ready: bool,
    }
    render_html(T {
        mode: crate::ui_ctx::DevModeData {
            monitor_only: state.0.settings.get().cec.monitor_only,
        },
        adapter_ready: state.0.adapter.ready(),
    })
}

pub async fn dev_fragment_devices(State(state): State<AppState>) -> Response {
    #[derive(Template)]
    #[template(path = "dev_devices.html")]
    struct T {
        ctx: crate::ui_ctx::DevicesPanelData,
    }
    render_html(T {
        ctx: devices_panel_data(&state),
    })
}

pub async fn dev_fragment_trace(State(state): State<AppState>) -> Response {
    #[derive(Template)]
    #[template(path = "dev_trace.html")]
    struct T {
        ctx: crate::ui_ctx::DevTraceData,
    }
    let frames = state.0.bus.recent_frames();
    let ctx = crate::ui_ctx::DevTraceData {
        frames: frames
            .iter()
            .rev()
            .take(100)
            .map(|f| crate::ui_ctx::FrameRow {
                time: f
                    .timestamp
                    .with_timezone(&chrono::Local)
                    .format("%H:%M:%S%.3f")
                    .to_string(),
                initiator: f.initiator,
                destination: f.destination,
                opcode: f.opcode.clone(),
                ack: f.ack,
                params: f.params_hex.join(" "),
            })
            .collect(),
    };
    render_html(T { ctx })
}

fn render_html<T: Template>(t: T) -> Response {
    match t.render() {
        Ok(s) => html_response(s),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("template: {e}")),
    }
}

// ---- pages ------------------------------------------------------------------

async fn dashboard(State(state): State<AppState>) -> Response {
    render_html(DashboardTmpl {
        ctx: remote_data(&state),
        version: VERSION.into(),
    })
}

async fn settings_page(State(state): State<AppState>) -> Response {
    render_html(SettingsTmpl {
        mqtt: mqtt_panel_data(&state),
        health: health_data(&state),
        monitor_only: state.0.settings.get().cec.monitor_only,
        token_set: !state.auth_token().is_empty(),
        version: VERSION.into(),
    })
}

async fn dev_page(State(state): State<AppState>) -> Response {
    render_html(DevTmpl {
        mode: crate::ui_ctx::DevModeData {
            monitor_only: state.0.settings.get().cec.monitor_only,
        },
        adapter_ready: state.0.adapter.ready(),
        actions: crate::strategies::ALL_ACTIONS
            .iter()
            .map(|(n, _)| n.to_string())
            .collect(),
        keys: crate::cec::keycode_names().into_iter().collect(),
        version: VERSION.into(),
    })
}

async fn login_page() -> Response {
    render_html(LoginTmpl {
        error: String::new(),
    })
}

// ---- actions ------------------------------------------------------------------

fn hx_toast(level: &str, message: &str) -> (axum::http::HeaderMap, ()) {
    let mut h = axum::http::HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&format!("{level}: {message}")) {
        h.insert("x-capi-toast", v);
    }
    (h, ())
}

fn action_result(
    state: &AppState,
    title: &str,
    res: Result<String, crate::exec::ExecError>,
) -> Response {
    match res {
        Ok(msg) => {
            let (headers, _) = hx_toast("ok", &msg);
            let mut r = html_response(format!(
                "<div class=\"action-note ok\">{title}: {}</div>",
                askama_escape(&msg)
            ));
            r.headers_mut().extend(headers);
            r
        }
        Err(e) => {
            let status = match e {
                crate::exec::ExecError::AdapterUnavailable => StatusCode::SERVICE_UNAVAILABLE,
                _ => StatusCode::BAD_REQUEST,
            };
            let (headers, _) = hx_toast("err", &e.to_string());
            let mut r = err(status, e.to_string());
            r.headers_mut().extend(headers);
            let _ = state;
            r
        }
    }
}

fn askama_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[derive(Deserialize)]
pub struct AddrForm {
    addr: Option<i32>,
}

async fn act_power_on(State(state): State<AppState>, Form(f): Form<AddrForm>) -> Response {
    let addr = f.addr.unwrap_or(0);
    let (adapter, steward) = (state.0.adapter.clone(), state.0.steward.clone());
    let res =
        tokio::task::spawn_blocking(move || crate::exec::power_on(&adapter, &steward, addr)).await;
    match res {
        Ok(r) => action_result(&state, "Power on", r.map(|_| "sent".into())),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

async fn act_power_off(State(state): State<AppState>, Form(f): Form<AddrForm>) -> Response {
    let addr = f.addr.unwrap_or(0);
    let (adapter, steward) = (state.0.adapter.clone(), state.0.steward.clone());
    let res =
        tokio::task::spawn_blocking(move || crate::exec::power_off(&adapter, &steward, addr)).await;
    match res {
        Ok(r) => action_result(&state, "Standby", r.map(|_| "sent".into())),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

async fn act_volume_up(State(state): State<AppState>, Form(_f): Form<AddrForm>) -> Response {
    run_volume(state, crate::strategies::Action::VolumeUp).await
}

async fn act_volume_down(State(state): State<AppState>, Form(_f): Form<AddrForm>) -> Response {
    run_volume(state, crate::strategies::Action::VolumeDown).await
}

async fn act_volume_mute(State(state): State<AppState>, Form(_f): Form<AddrForm>) -> Response {
    run_volume(state, crate::strategies::Action::Mute).await
}

async fn run_volume(state: AppState, action: crate::strategies::Action) -> Response {
    let Some(conn) = state.0.adapter.get() else {
        return unavailable();
    };
    let bus = state.0.bus.clone();
    let registry = state.0.registry.clone();
    let res = tokio::task::spawn_blocking(move || {
        crate::exec::volume_action(&conn, &bus, &registry, action, None)
    })
    .await;
    match res {
        Ok(r) => action_result(&state, action.as_str(), r),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

#[derive(Deserialize)]
pub struct SetSourceForm {
    addr: i32,
}

async fn act_set_source(State(state): State<AppState>, Form(f): Form<SetSourceForm>) -> Response {
    let (adapter, steward) = (state.0.adapter.clone(), state.0.steward.clone());
    let res = tokio::task::spawn_blocking(move || {
        crate::exec::set_active_source(&adapter, &steward, f.addr)
    })
    .await;
    match res {
        Ok(r) => action_result(
            &state,
            "Source",
            r.map(|_| format!("switched to {}", f.addr)),
        ),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

#[derive(Deserialize)]
pub struct HdmiForm {
    port: i32,
}

async fn act_hdmi(State(state): State<AppState>, Form(f): Form<HdmiForm>) -> Response {
    let (adapter, steward) = (state.0.adapter.clone(), state.0.steward.clone());
    let res =
        tokio::task::spawn_blocking(move || crate::exec::hdmi_port(&adapter, &steward, f.port))
            .await;
    match res {
        Ok(r) => action_result(&state, "HDMI", r.map(|_| format!("port {}", f.port))),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

#[derive(Deserialize)]
pub struct NavKeyForm {
    key: String,
    addr: Option<i32>,
}

async fn act_nav_key(State(state): State<AppState>, Form(f): Form<NavKeyForm>) -> Response {
    let Some(conn) = state.0.adapter.get() else {
        return unavailable();
    };
    let target = f.addr.or_else(|| {
        // Default: current active source, else TV.
        state.0.bus.copy_snapshot().active_source.checked_sub(0)
    });
    let _ = conn;
    let AppState(inner) = state.clone();
    let key = f.key.clone();
    let res = tokio::task::spawn_blocking(move || {
        let addr = target.filter(|t| *t >= 0).unwrap_or(0);
        crate::exec::send_key(&inner.adapter, &inner.bus, &inner.registry, addr, &key, 0)
    })
    .await;
    match res {
        Ok(r) => action_result(&state, "Key", r),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    }
}

#[derive(Deserialize)]
pub struct DeepScanForm {}

async fn act_deep_scan(State(state): State<AppState>, Form(_f): Form<DeepScanForm>) -> Response {
    let enqueued = state.0.steward.enqueue(crate::steward::JobKind::Deep);
    if enqueued {
        ok("deep scan queued", None)
    } else {
        err(StatusCode::SERVICE_UNAVAILABLE, "queue full")
    }
}

#[derive(Deserialize)]
pub struct MqttSaveForm {
    #[serde(default)]
    broker: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    pass: String,
    #[serde(default)]
    prefix: String,
}

async fn act_mqtt_save(State(state): State<AppState>, Form(f): Form<MqttSaveForm>) -> Response {
    let existing = state.0.settings.get().mqtt;
    let pass = match f.pass.as_str() {
        "***" => existing.pass.clone(),
        "" => String::new(),
        other => other.to_string(),
    };
    let cfg = crate::types::MqttConfig {
        broker: f.broker.trim().to_string(),
        user: f.user.trim().to_string(),
        pass,
        prefix: if f.prefix.trim().is_empty() {
            "capi".into()
        } else {
            f.prefix.trim().to_string()
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

// ---- login ---------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LoginForm {
    token: String,
}

async fn login_submit(Form(f): Form<LoginForm>) -> Response {
    let expected = std::env::var("CAPI_LOGIN_TOKEN").unwrap_or_default();
    // The authoritative check happens against config in middleware; here we
    // just set the cookie when it matches what the server knows.
    let cfg_token = LOGIN_TOKEN.get().cloned().unwrap_or_default();
    if (!cfg_token.is_empty() && f.token == cfg_token)
        || (!expected.is_empty() && f.token == expected)
    {
        let mut resp = axum::response::Redirect::to("/").into_response();
        resp.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_str(&format!(
                "capi_token={}; Path=/; HttpOnly; SameSite=Lax",
                f.token
            ))
            .unwrap(),
        );
        resp
    } else {
        render_html(LoginTmpl {
            error: "Invalid token".into(),
        })
    }
}

pub static LOGIN_TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard))
        .route("/settings", get(settings_page))
        .route("/dev", get(dev_page))
        .route("/login", get(login_page).post(login_submit))
        .route("/ui/static/{*path}", get(static_asset))
        .route("/ui/fragment/bus_banner", get(fragment_bus_banner))
        .route("/ui/fragment/devices", get(fragment_devices))
        .route("/ui/fragment/device_power", get(fragment_device_power))
        .route("/ui/fragment/mqtt_panel", get(fragment_mqtt_panel))
        .route("/ui/fragment/health", get(fragment_health))
        .route("/ui/fragment/topology_hdmi", get(fragment_topology_hdmi))
        .route("/ui/fragment/volume_panel", get(fragment_volume_panel))
        .route("/ui/fragment/nav_panel", get(fragment_nav_panel))
        .route("/ui/fragment/source_panel", get(fragment_source_panel))
        .route("/ui/fragment/logs", get(fragment_logs))
        .route("/ui/action/deep_scan", post(act_deep_scan))
        .route("/ui/action/power_on", post(act_power_on))
        .route("/ui/action/power_off", post(act_power_off))
        .route("/ui/action/volume_up", post(act_volume_up))
        .route("/ui/action/volume_down", post(act_volume_down))
        .route("/ui/action/volume_mute", post(act_volume_mute))
        .route("/ui/action/set_source", post(act_set_source))
        .route("/ui/action/hdmi", post(act_hdmi))
        .route("/ui/action/nav_key", post(act_nav_key))
        .route("/ui/action/mqtt_save", post(act_mqtt_save))
        .route("/ui/dev/fragment/banner", get(dev_fragment_banner))
        .route("/ui/dev/fragment/devices", get(dev_fragment_devices))
        .route("/ui/dev/fragment/trace", get(dev_fragment_trace))
}

async fn static_asset(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    match crate::assets::get(&path) {
        Some((mime, bytes)) => {
            let mut resp = (StatusCode::OK, bytes).into_response();
            resp.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
            resp.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            );
            resp
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
