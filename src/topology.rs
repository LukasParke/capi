//! HDMI topology derived from the steward snapshot — never from live bus
//! probes on request paths (fixes head-of-line blocking).

use crate::busstate::BusState;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct TopologyPortRow {
    pub port: i64,
    pub devices: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopologyPayload {
    pub own_addresses: Vec<i32>,
    pub known_port_count: i64,
    pub ports: Vec<TopologyPortRow>,
}

/// Physical address encoding: 4 nibbles a.b.c.d; the display sits at 0.0.0.0
/// and its ports are d=1..15, so port = low nibble for direct children.
fn port_of(phys: &str) -> Option<i64> {
    let parts: Vec<&str> = phys.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let nibbles = [parts[0], parts[1], parts[2], parts[3]];
    if nibbles[0] == "0" && nibbles[1] == "0" && nibbles[2] == "0" {
        return nibbles[3].parse().ok();
    }
    // Deeper nodes report their parent's port as the second-lowest nibble.
    nibbles[2].parse().ok()
}

pub fn build_from_snapshot(bus: &BusState) -> TopologyPayload {
    let snap = bus.copy_snapshot();
    let mut ports: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    let mut own_addresses = Vec::new();
    let mut max_port = 0i64;

    for d in &snap.devices {
        let la = d
            .get("logical_address")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        let name = d
            .get("osd_name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                d.get("address_name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("LA {la}"));
        let is_own = d.get("is_own").and_then(|v| v.as_bool()).unwrap_or(false);
        if is_own {
            own_addresses.push(la as i32);
        }
        if let Some(phys) = d.get("physical_address").and_then(|v| v.as_str()) {
            if let Some(port) = port_of(phys) {
                max_port = max_port.max(port);
                ports.entry(port).or_default().push(name);
            }
        }
    }

    // Guarantee at least 4 port buttons in the UI (parity with remoteHDMIDefault).
    let known = max_port.max(4);
    let rows = (1..=known)
        .map(|p| TopologyPortRow {
            port: p,
            devices: ports.remove(&p).unwrap_or_default(),
        })
        .collect();

    TopologyPayload {
        own_addresses,
        known_port_count: known,
        ports: rows,
    }
}
