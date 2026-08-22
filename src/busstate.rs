//! Cached bus snapshot, passively-observed device fields, and the frame ring.
//!
//! Fix vs Go: frame ring entries carry monotonic sequence numbers; diffs are
//! computed by sequence, never by index position, so a wrapped ring can no
//! longer hide real replies (the diffFrames misclassification bug).

use crate::types::BusFrameEntry;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, serde::Serialize)]
pub struct BusStateSnapshot {
    pub devices: Vec<serde_json::Value>,
    pub logical_addresses: Vec<i32>,
    pub active_source: i32,
    pub cec_ready: bool,
    pub monitoring: bool,
    pub scan_in_progress: bool,
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_full_scan_at: Option<DateTime<Utc>>,
    pub stale_threshold_sec: i64,
    pub generation: i64,
}

struct Observed {
    fields: HashMap<String, serde_json::Value>,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
}

#[derive(Default)]
struct Inner {
    devices: Vec<serde_json::Value>,
    addresses: Vec<i32>,
    active_source: i64,
    cec_ready: bool,
    monitoring: bool,
    scan_in_progress: bool,
    last_full_scan_at: Option<DateTime<Utc>>,
    stale_threshold_sec: i64,
    observed: HashMap<i32, Observed>,
    ring: VecDeque<(u64, BusFrameEntry)>,
    ring_cap: usize,
}

pub struct BusState {
    inner: Mutex<Inner>,
    generation: AtomicI64,
    next_seq: AtomicU64,
    /// Frames delivered to the ring since boot (metrics).
    pub frames_captured: AtomicU64,
}

impl Default for BusState {
    fn default() -> Self {
        Self::new()
    }
}

impl BusState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                active_source: -1,
                stale_threshold_sec: 180,
                ..Default::default()
            }),
            generation: AtomicI64::new(0),
            next_seq: AtomicU64::new(1),
            frames_captured: AtomicU64::new(0),
        }
    }

    pub fn set_cec_ready(&self, on: bool) {
        self.inner.lock().expect("bus lock").cec_ready = on;
    }

    #[allow(dead_code)]
    pub fn set_monitoring(&self, on: bool) {
        self.inner.lock().expect("bus lock").monitoring = on;
    }

    pub fn set_scan_in_progress(&self, v: bool) {
        self.inner.lock().expect("bus lock").scan_in_progress = v;
    }

    pub fn set_frame_ring_capacity(&self, cap: usize) {
        let mut g = self.inner.lock().expect("bus lock");
        g.ring_cap = cap;
        if cap == 0 {
            g.ring.clear();
        }
    }

    pub fn frame_ring_capacity(&self) -> usize {
        self.inner.lock().expect("bus lock").ring_cap
    }

    pub fn bump_generation(&self) -> i64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    #[allow(clippy::too_many_arguments)]
    pub fn replace_snapshot(
        &self,
        devices: Vec<serde_json::Value>,
        addresses: Vec<i32>,
        active_source: i32,
        cec_ready: bool,
        monitoring: bool,
        last_full: Option<DateTime<Utc>>,
        stale_threshold_sec: i64,
        ring_cap: usize,
    ) {
        let mut g = self.inner.lock().expect("bus lock");
        g.devices = devices;
        g.addresses = addresses;
        g.active_source = active_source as i64;
        g.cec_ready = cec_ready;
        g.monitoring = monitoring;
        g.scan_in_progress = false;
        if last_full.is_some() {
            g.last_full_scan_at = last_full;
        }
        g.stale_threshold_sec = stale_threshold_sec;
        g.ring_cap = ring_cap;
    }

    pub fn update_active_source_quick(&self, active: i32, cec_ready: bool) {
        let mut g = self.inner.lock().expect("bus lock");
        g.active_source = active as i64;
        g.cec_ready = cec_ready;
    }

    pub fn copy_snapshot(&self) -> BusStateSnapshot {
        let g = self.inner.lock().expect("bus lock");
        BusStateSnapshot {
            devices: g.devices.clone(),
            logical_addresses: g.addresses.clone(),
            active_source: g.active_source as i32,
            cec_ready: g.cec_ready,
            monitoring: g.monitoring,
            scan_in_progress: g.scan_in_progress,
            stale: g
                .last_full_scan_at
                .map(|t| Utc::now() - t > chrono::Duration::seconds(g.stale_threshold_sec.max(1)))
                .unwrap_or(true),
            last_full_scan_at: g.last_full_scan_at,
            stale_threshold_sec: g.stale_threshold_sec,
            generation: self.generation.load(Ordering::Relaxed),
        }
    }

    // ---- observed (passive) state --------------------------------------

    pub fn record_observed(&self, addr: i32, key: &str, value: serde_json::Value) {
        let mut g = self.inner.lock().expect("bus lock");
        let now = Utc::now();
        let o = g.observed.entry(addr).or_insert_with(|| Observed {
            fields: HashMap::new(),
            first_seen: now,
            last_seen: now,
        });
        o.fields.insert(key.to_string(), value);
        o.last_seen = now;
    }

    pub fn note_seen(&self, addr: i32) {
        let mut g = self.inner.lock().expect("bus lock");
        let now = Utc::now();
        let o = g.observed.entry(addr).or_insert_with(|| Observed {
            fields: HashMap::new(),
            first_seen: now,
            last_seen: now,
        });
        o.last_seen = now;
    }

    pub fn observed_addresses(&self) -> Vec<i32> {
        self.inner
            .lock()
            .expect("bus lock")
            .observed
            .keys()
            .copied()
            .collect()
    }

    pub fn seen_timestamps(&self) -> (HashMap<i32, DateTime<Utc>>, HashMap<i32, DateTime<Utc>>) {
        let g = self.inner.lock().expect("bus lock");
        (
            g.observed.iter().map(|(k, v)| (*k, v.first_seen)).collect(),
            g.observed.iter().map(|(k, v)| (*k, v.last_seen)).collect(),
        )
    }

    pub fn prune_stale_observed(&self, ttl: Duration) {
        if ttl.is_zero() {
            return;
        }
        let cutoff = Utc::now()
            - chrono::Duration::from_std(ttl).unwrap_or_else(|_| chrono::Duration::minutes(10));
        let mut g = self.inner.lock().expect("bus lock");
        g.observed.retain(|_, v| v.last_seen > cutoff);
    }

    /// Fold observed fields into freshly built device maps (steward path).
    pub fn merge_observed_into_devices(&self, devices: &mut [serde_json::Value]) {
        let g = self.inner.lock().expect("bus lock");
        for d in devices.iter_mut() {
            let Some(obj) = d.as_object_mut() else {
                continue;
            };
            let Some(la) = obj.get("logical_address").and_then(|v| v.as_i64()) else {
                continue;
            };
            if let Some(o) = g.observed.get(&(la as i32)) {
                for (k, v) in &o.fields {
                    obj.insert(format!("observed_{k}"), v.clone());
                }
                obj.insert(
                    "observed_at".into(),
                    serde_json::json!(o.last_seen.to_rfc3339()),
                );
            }
        }
    }

    /// Update passive fields from a command seen on the bus.
    pub fn apply_observed_command(&self, cmd: &crate::cec::Command) {
        use crate::cec::Opcode;
        let initiator = cmd.initiator.0 as i32;
        match cmd.opcode {
            Opcode::REPORT_POWER_STATUS => {
                if let Some(p) = cmd.parameters.first() {
                    let status = crate::cec::power_status_str(*p);
                    self.record_observed(initiator, "power_status", serde_json::json!(status));
                }
            }
            Opcode::REPORT_AUDIO_STATUS => {
                if let Some(p) = cmd.parameters.first() {
                    let raw = *p;
                    self.record_observed(
                        initiator,
                        "audio_volume_raw",
                        serde_json::json!(raw & 0x7F),
                    );
                    self.record_observed(
                        initiator,
                        "audio_muted",
                        serde_json::json!(raw & 0x80 != 0),
                    );
                }
            }
            Opcode::SET_OSD_NAME => {
                let name = String::from_utf8_lossy(&cmd.parameters)
                    .trim_end_matches('\0')
                    .to_string();
                if !name.is_empty() {
                    self.record_observed(initiator, "osd_name_fragment", serde_json::json!(name));
                }
            }
            Opcode::FEATURE_ABORT => {
                if !cmd.parameters.is_empty() {
                    self.record_observed(
                        initiator,
                        "last_feature_abort_opcode",
                        serde_json::json!(cmd.parameters[0]),
                    );
                    if cmd.parameters.len() > 1 {
                        self.record_observed(
                            initiator,
                            "last_feature_abort_reason",
                            serde_json::json!(cmd.parameters[1]),
                        );
                    }
                }
            }
            Opcode::DEVICE_VENDOR_ID if cmd.parameters.len() >= 3 => {
                let v = ((cmd.parameters[0] as u32) << 16)
                    | ((cmd.parameters[1] as u32) << 8)
                    | cmd.parameters[2] as u32;
                self.record_observed(
                    initiator,
                    "vendor_id",
                    serde_json::json!(format!("0x{v:06x}")),
                );
            }
            _ => {}
        }
    }
    pub fn append_frame(&self, cmd: &crate::cec::Command, cap: usize) {
        if cap == 0 {
            return;
        }
        let entry = BusFrameEntry {
            timestamp: Utc::now(),
            initiator: cmd.initiator.0 as i32,
            destination: cmd.destination.0 as i32,
            opcode: format!("0x{:02X}", cmd.opcode.0),
            ack: cmd.ack,
            eom: cmd.eom,
            opcode_set: cmd.opcode_set,
            params_hex: cmd.parameters.iter().map(|b| format!("{b:02X}")).collect(),
        };
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        {
            let mut g = self.inner.lock().expect("bus lock");
            g.ring.push_back((seq, entry));
            while g.ring.len() > cap {
                g.ring.pop_front();
            }
        }
        self.frames_captured.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot of the current ring.
    pub fn recent_frames(&self) -> Vec<BusFrameEntry> {
        self.inner
            .lock()
            .expect("bus lock")
            .ring
            .iter()
            .map(|(_, e)| e.clone())
            .collect()
    }

    /// Highest sequence number currently in the ring.
    pub fn ring_high_water(&self) -> u64 {
        self.inner
            .lock()
            .expect("bus lock")
            .ring
            .back()
            .map(|(s, _)| *s)
            .unwrap_or(0)
    }

    /// Frames with seq strictly greater than `after` — the wrap-safe diff.
    pub fn frames_after(&self, after: u64) -> Vec<BusFrameEntry> {
        self.inner
            .lock()
            .expect("bus lock")
            .ring
            .iter()
            .filter(|(s, _)| *s > after)
            .map(|(_, e)| e.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cec::{Command, LogicalAddress, Opcode};

    fn state() -> BusState {
        let s = BusState::new();
        s.set_frame_ring_capacity(4);
        s
    }

    fn cmd(op: Opcode) -> Command {
        Command {
            initiator: LogicalAddress(4),
            destination: LogicalAddress(0),
            opcode: op,
            opcode_set: true,
            parameters: vec![1, 2],
            ack: true,
            eom: true,
        }
    }

    #[test]
    fn snapshot_roundtrip_and_defaults() {
        let s = state();
        let snap = s.copy_snapshot();
        assert!(!snap.cec_ready);
        assert_eq!(snap.active_source, -1);
        assert!(snap.stale); // never scanned

        s.set_cec_ready(true);
        s.set_scan_in_progress(true);
        s.replace_snapshot(
            vec![serde_json::json!({"logical_address": 4})],
            vec![4],
            4,
            true,
            false,
            Some(chrono::Utc::now()),
            180,
            256,
        );
        let snap = s.copy_snapshot();
        assert!(snap.cec_ready);
        assert!(!snap.scan_in_progress); // replace clears the flag
        assert!(!snap.stale);
        assert_eq!(snap.devices[0]["logical_address"], 4);
    }

    #[test]
    fn frame_ring_trims_to_cap_and_diff_by_seq_survives_wrap() {
        // Regression: the Go diffFrames compared index positions and silently
        // lost replies once the ring wrapped. Sequence numbers must not.
        let s = state();
        for _ in 0..10 {
            s.append_frame(&cmd(Opcode::ACTIVE_SOURCE), 4); // ring holds 4
        }
        assert_eq!(s.recent_frames().len(), 4);

        let pre = s.ring_high_water();
        s.append_frame(&cmd(Opcode::REPORT_POWER_STATUS), 4);
        let new = s.frames_after(pre);
        assert_eq!(new.len(), 1, "exactly the post-snapshot frame");
        assert_eq!(new[0].opcode, "0x90");
    }

    #[test]
    fn ring_disabled_drops_everything() {
        let s = state();
        s.set_frame_ring_capacity(0);
        s.append_frame(&cmd(Opcode::STANDBY), 0);
        assert!(s.recent_frames().is_empty());
        assert_eq!(s.ring_high_water(), 0);
    }

    #[test]
    fn observed_fields_merge_into_devices() {
        let s = state();
        s.note_seen(9);
        s.record_observed(9, "vendor_id", serde_json::json!("0x809819"));
        let mut devices = vec![serde_json::json!({"logical_address": 9})];
        s.merge_observed_into_devices(&mut devices);
        assert_eq!(devices[0]["observed_vendor_id"], "0x809819");
        assert!(devices[0]["observed_at"].is_string());
        // first/last seen timestamps annotated
        let (first, last) = s.seen_timestamps();
        assert!(first.contains_key(&9) && last.contains_key(&9));
    }

    #[test]
    fn prune_drops_stale_but_keeps_fresh() {
        let s = state();
        s.note_seen(5);
        // TTL zero disables pruning.
        s.prune_stale_observed(Duration::ZERO);
        assert!(s.observed_addresses().contains(&5));
        // A negative-equivalent tiny TTL prunes (last_seen is now, so use 0s).
        s.prune_stale_observed(Duration::from_nanos(1));
        std::thread::sleep(Duration::from_millis(5));
        s.prune_stale_observed(Duration::from_millis(2));
        assert!(!s.observed_addresses().contains(&5));
    }

    #[test]
    fn apply_observed_command_extracts_fields() {
        let s = state();
        s.apply_observed_command(&Command {
            initiator: LogicalAddress(4),
            destination: LogicalAddress(15),
            opcode: Opcode::DEVICE_VENDOR_ID,
            opcode_set: true,
            parameters: vec![0x00, 0x80, 0x19],
            ack: false,
            eom: true,
        });
        s.apply_observed_command(&Command {
            initiator: LogicalAddress(4),
            destination: LogicalAddress(15),
            opcode: Opcode::REPORT_POWER_STATUS,
            opcode_set: true,
            parameters: vec![0x00],
            ack: false,
            eom: true,
        });
        s.apply_observed_command(&Command {
            initiator: LogicalAddress(4),
            destination: LogicalAddress(15),
            opcode: Opcode::FEATURE_ABORT,
            opcode_set: true,
            parameters: vec![0x44, 0x01],
            ack: false,
            eom: true,
        });
        let snap = s.copy_snapshot();
        // observed fields live in the store, not the snapshot devices
        s.merge_observed_into_devices(&mut []);
        let _ = snap;
        let mut d = vec![serde_json::json!({"logical_address": 4})];
        s.merge_observed_into_devices(&mut d);
        assert_eq!(d[0]["observed_vendor_id"], "0x008019");
        assert_eq!(d[0]["observed_power_status"], "on");
        assert_eq!(d[0]["observed_last_feature_abort_opcode"], 0x44);
    }
}
