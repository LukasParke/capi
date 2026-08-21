//! Shared domain types: wire events, JSON envelope, persisted config shapes.
//!
//! Wire compatibility with the original Go service is intentional: event
//! payloads, the `{status,message,data}` envelope, and `config.json` field
//! names are unchanged so existing clients and config files keep working.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Canonical event type strings published on SSE/WS/MQTT.
#[allow(dead_code)]
pub mod event_type {
    pub const POWER_CHANGE: &str = "power_change";
    pub const SOURCE_ACTIVATED: &str = "source_activated";
    pub const KEY_PRESS: &str = "key_press";
    pub const COMMAND: &str = "command";
    pub const ALERT: &str = "alert";
    pub const DEVICES_CHANGED: &str = "devices_changed";
    pub const CONFIGURATION_CHANGED: &str = "configuration_changed";
    pub const ADAPTER_STATE: &str = "adapter_state";
}

/// One event on the hub. Wire format matches the Go `CECEvent`:
/// `{"type":"...","timestamp":"...","data":{...}}`.
#[derive(Debug, Clone, Serialize)]
pub struct AppEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub timestamp: DateTime<Utc>,
    pub data: serde_json::Value,
}

impl AppEvent {
    pub fn new(kind: &str, data: serde_json::Value) -> Self {
        Self {
            kind: kind.to_string(),
            timestamp: Utc::now(),
            data,
        }
    }
}

/// Standard JSON envelope for every /api handler:
/// `{"status":"success"|"error","message":"...","data":...}`.
///
/// Fix vs Go: the ad-hoc `"accepted"` status is gone; async acceptance uses
/// `success` with an explicit `accepted: true` field inside `data`.
#[derive(Debug, Serialize)]
pub struct Envelope {
    pub status: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Envelope {
    pub fn success(message: impl Into<String>, data: Option<serde_json::Value>) -> Self {
        Self {
            status: "success",
            message: message.into(),
            data,
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            status: "error",
            message: message.into(),
            data: None,
        }
    }
}

/// One captured CEC frame (frame ring). Field names match Go `BusFrameEntry`.
#[derive(Debug, Clone, Serialize)]
pub struct BusFrameEntry {
    pub timestamp: DateTime<Utc>,
    pub initiator: i32,
    pub destination: i32,
    /// Lowercase hex opcode with 0x prefix, e.g. "0x90".
    pub opcode: String,
    pub ack: bool,
    pub eom: bool,
    pub opcode_set: bool,
    pub params_hex: Vec<String>,
}

/// Per-vendor probe tweaks keyed by lowercase 0x-prefixed vendor id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VendorProfile {
    #[serde(default)]
    pub skip_probes: Vec<String>,
    #[serde(default)]
    pub settle_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttConfig {
    #[serde(default)]
    pub broker: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub pass: String,
    #[serde(default)]
    pub prefix: String,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            broker: String::new(),
            user: String::new(),
            pass: String::new(),
            prefix: "capi".into(),
        }
    }
}

/// Bus-disruption knobs applied to every libcec session open.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CecConfig {
    #[serde(default)]
    pub monitor_only: bool,
    #[serde(default)]
    pub activate_source: bool,
    #[serde(default)]
    pub wake_on_connect: Vec<i32>,
    #[serde(default)]
    pub power_off_on_disconnect: Vec<i32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub strategy_overrides: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BusConfig {
    #[serde(default)]
    pub reconcile_interval_sec: i64,
    #[serde(default)]
    pub deep_settle_ms: i64,
    #[serde(default)]
    pub rescan_extra_settle_ms: i64,
    #[serde(default)]
    pub stale_threshold_sec: i64,
    #[serde(default)]
    pub stale_device_ttl_sec: i64,
    #[serde(default)]
    pub frame_ring_size: i64,
    /// None = follow CLI only; Some(v) overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<bool>,
    #[serde(default)]
    pub vendor_profiles: BTreeMap<String, VendorProfile>,
}

impl BusConfig {
    #[allow(dead_code)]
    #[allow(dead_code)]
    pub fn reconcile_interval(&self) -> chrono::Duration {
        if self.reconcile_interval_sec <= 0 {
            chrono::Duration::seconds(60)
        } else {
            chrono::Duration::seconds(self.reconcile_interval_sec)
        }
    }
    pub fn reconcile_interval_std(&self) -> std::time::Duration {
        if self.reconcile_interval_sec <= 0 {
            std::time::Duration::from_secs(60)
        } else {
            std::time::Duration::from_secs(self.reconcile_interval_sec as u64)
        }
    }
    pub fn deep_settle(&self) -> std::time::Duration {
        if self.deep_settle_ms <= 0 {
            std::time::Duration::from_millis(2500)
        } else {
            std::time::Duration::from_millis(self.deep_settle_ms as u64)
        }
    }
    pub fn rescan_extra_settle(&self) -> std::time::Duration {
        if self.rescan_extra_settle_ms < 0 {
            std::time::Duration::ZERO
        } else {
            std::time::Duration::from_millis(self.rescan_extra_settle_ms.max(0) as u64)
        }
    }
    #[allow(dead_code)]
    pub fn stale_threshold(&self) -> std::time::Duration {
        if self.stale_threshold_sec <= 0 {
            std::time::Duration::from_secs(180)
        } else {
            std::time::Duration::from_secs(self.stale_threshold_sec as u64)
        }
    }
    /// 0/unset maps to 256; negative disables the ring entirely.
    pub fn frame_ring_size(&self) -> usize {
        if self.frame_ring_size < 0 {
            0
        } else if self.frame_ring_size == 0 {
            256
        } else {
            self.frame_ring_size as usize
        }
    }
    /// Negative disables pruning; 0/unset maps to 10 minutes.
    pub fn stale_device_ttl(&self) -> std::time::Duration {
        if self.stale_device_ttl_sec < 0 {
            std::time::Duration::ZERO
        } else if self.stale_device_ttl_sec == 0 {
            std::time::Duration::from_secs(600)
        } else {
            std::time::Duration::from_secs(self.stale_device_ttl_sec as u64)
        }
    }
}

/// On-disk `config.json`. Field names identical to the Go service.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub mqtt: MqttConfig,
    #[serde(default)]
    pub bus: BusConfig,
    #[serde(default)]
    pub cec: CecConfig,
    /// Optional bearer token protecting /api and /ui actions. When empty the
    /// API is open on the LAN (legacy behavior); a warning is logged.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub auth_token: String,
}
