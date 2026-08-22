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
pub fn port_of(phys: &str) -> Option<i64> {
    // CEC physical address a.b.c.d: the TV input for any downstream device is
    // the leftmost nonzero nibble among a..c (d is only meaningful for the
    // device's own inputs).
    let parts: Vec<&str> = phys.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    for nib in &parts[..3] {
        if let Ok(v) = nib.parse::<i64>() {
            if v > 0 {
                return Some(v);
            }
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::busstate::BusState;

    #[test]
    fn port_of_parses_direct_children_and_deeper_nodes() {
        assert_eq!(port_of("1.0.0.0"), Some(1));
        assert_eq!(port_of("5.0.0.0"), Some(5));
        assert_eq!(port_of("2.1.0.0"), Some(2)); // downstream of TV input 2
        assert_eq!(port_of("garbage"), None);
        assert_eq!(port_of("1.0.0"), None);
    }

    #[test]
    fn topology_guarantees_minimum_four_ports() {
        let bus = BusState::new();
        bus.replace_snapshot(
            vec![serde_json::json!({
                "logical_address": 4,
                "osd_name": "Box",
                "physical_address": "2.0.0.0",
                "is_own": false,
            })],
            vec![4],
            -1,
            true,
            false,
            None,
            180,
            0,
        );
        let topo = build_from_snapshot(&bus);
        assert!(topo.known_port_count >= 4);
        assert_eq!(topo.ports.len() as i64, topo.known_port_count);
        let port2 = topo.ports.iter().find(|p| p.port == 2).unwrap();
        assert_eq!(port2.devices, vec!["Box".to_string()]);
        // Own addresses picked up when flagged.
        assert!(topo.own_addresses.is_empty()); // is_own=false above
    }
}
