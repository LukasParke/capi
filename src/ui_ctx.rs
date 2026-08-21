#![allow(dead_code)]
//! Template context structs (askama). The templates in `templates/` compile
//! against these; field names are the template variable names.

use crate::busstate::BusStateSnapshot;

#[derive(serde::Serialize)]
pub struct PageShell {
    pub title: String,
    pub version: String,
    pub active_nav: &'static str,
}

/// Status banner above everything: adapter/session health.
pub struct BusBannerData {
    pub cec_ready: bool,
    pub scan_in_progress: bool,
    pub stale: bool,
    pub monitoring: bool,
    pub last_full_scan: String,
    pub active_source: i32,
    pub device_count: usize,
}

impl BusBannerData {
    pub fn from_snapshot(s: &BusStateSnapshot) -> Self {
        Self {
            cec_ready: s.cec_ready,
            scan_in_progress: s.scan_in_progress,
            stale: s.stale,
            monitoring: s.monitoring,
            last_full_scan: s
                .last_full_scan_at
                .map(|t| {
                    t.with_timezone(&chrono::Local)
                        .format("%H:%M:%S")
                        .to_string()
                })
                .unwrap_or_else(|| "never".into()),
            active_source: s.active_source,
            device_count: s.devices.len(),
        }
    }
}

pub struct DeviceRow {
    pub logical_address: i32,
    pub display_name: String,
    pub address_name: String,
    pub role: String,
    pub physical_address: String,
    pub hdmi_port: i64,
    pub vendor_name: String,
    pub vendor_id: String,
    pub cec_version: String,
    pub power_status: String,
    pub power_observed_at: String,
    pub discovery: String,
    pub is_own: bool,
    pub is_active_source: bool,
    pub is_audio_system: bool,
    pub is_ghost: bool,
    pub first_seen: String,
    pub last_seen: String,
}

pub struct DevicesPanelData {
    pub devices: Vec<DeviceRow>,
    pub message: String,
}

pub struct HdmiPortButton {
    pub port: i64,
    pub selected: bool,
}

pub struct NavTarget {
    pub la: i32,
    pub label: String,
    pub selected: bool,
}

#[allow(dead_code)]
pub struct RemoteData {
    pub banner: BusBannerData,
    pub devices: Vec<DeviceRow>,
    pub hdmi_ports: Vec<HdmiPortButton>,
    pub nav_targets: Vec<NavTarget>,
    pub audio_display_volume: i32,
    pub audio_muted: bool,
    pub audio_available: bool,
}

pub struct MqttPanelData {
    pub broker: String,
    pub user: String,
    pub prefix: String,
    pub pass_set: bool,
    pub connected: bool,
}

pub struct HealthData {
    pub version: String,
    pub uptime: String,
    pub cec_ready: bool,
    pub lib_info: String,
    pub subscribers: usize,
    pub events_dropped: u64,
    pub frames_captured: u64,
}

pub struct LogLine {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

pub struct LogsData {
    pub lines: Vec<LogLine>,
}

pub struct TopologyPortRow {
    pub port: i64,
    pub devices: Vec<String>,
}

pub struct TopologyData {
    pub own_addresses: Vec<i32>,
    pub known_port_count: i64,
    pub ports: Vec<TopologyPortRow>,
}

// ---- dev console ----

pub struct DevModeData {
    pub monitor_only: bool,
}

pub struct DevProbeStep {
    pub name: String,
    pub opcode: String,
    pub result: String,
    pub error: String,
    pub elapsed_ms: i64,
    pub replies: Vec<String>,
}

pub struct DevProbeResult {
    pub address: i32,
    pub kind: String,
    pub total_replies: usize,
    pub steps: Vec<DevProbeStep>,
}

pub struct DevStrategyResult {
    pub strategy: String,
    pub status: String,
    pub acked: bool,
    pub reply_name: String,
    pub abort_opcode: i32,
    pub elapsed_ms: i64,
    pub error: String,
}

pub struct DevActionResult {
    pub ok: bool,
    pub title: String,
    pub detail: String,
    pub strategies: Vec<DevStrategyResult>,
    pub raw_json: String,
}

pub struct FrameRow {
    pub time: String,
    pub initiator: i32,
    pub destination: i32,
    pub opcode: String,
    pub ack: bool,
    pub params: String,
}

pub struct DevTraceData {
    pub frames: Vec<FrameRow>,
}

pub struct EventFeedEntry {
    pub time: String,
    pub kind: String,
    pub summary: String,
}
