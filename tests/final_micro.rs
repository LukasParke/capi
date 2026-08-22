//! Micro-gap closure: settings edges, busstate monitoring, topology fallback,
//! util duration units.

use capi::{topology, BusState};
use std::time::Duration;

#[test]
fn settings_empty_file_yields_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.json");
    std::fs::write(&p, "").unwrap();
    let (s, _) = capi::settings::Settings::load(&p).unwrap();
    assert_eq!(s.get().mqtt.prefix, "capi");
}

#[test]
fn quarantine_missing_file_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("none.json");
    capi::settings::Settings::quarantine_corrupt(&p);
    assert!(!p.exists());
}

#[test]
fn busstate_monitoring_toggle() {
    let bus = BusState::new();
    bus.set_monitoring(true);
    assert!(bus.copy_snapshot().monitoring);
    bus.set_monitoring(false);
    assert!(!bus.copy_snapshot().monitoring);
}

#[tokio::test]
async fn topology_uses_address_name_when_no_osd() {
    let bus = BusState::new();
    bus.replace_snapshot(
        vec![serde_json::json!({
            "logical_address": 4,
            "address_name": "PlaybackDevice1",
            "physical_address": "3.0.0.0",
            "is_own": false,
        })],
        vec![4],
        -1,
        false,
        false,
        None,
        180,
        0,
    );
    let topo = topology::build_from_snapshot(&bus);
    let port3 = topo.ports.iter().find(|p| p.port == 3).unwrap();
    assert!(!port3.devices.is_empty());
}

#[test]
fn parse_wait_minutes_and_invalid_unit() {
    use capi::util;
    assert_eq!(util::parse_wait("2m").unwrap(), Duration::from_secs(120));
    assert_eq!(util::parse_wait("90s").unwrap(), Duration::from_secs(90));
    assert!(util::parse_wait("5x").is_err());
}
