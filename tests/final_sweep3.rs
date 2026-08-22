//! Final sweep 3: settings save-failure, exec/steward/types edges
//! (feature = "mock-cec").

#![cfg(feature = "mock-cec")]

use axum::body::Body;
use capi::cec;
use capi::cec::LogicalAddress;
use capi::events::EventHub;
use capi::steward::Steward;
use capi::{AdapterHandle, BusState};
mod common;
use serial_test::serial;
use std::sync::Arc;
use tower::ServiceExt;

// -- settings save-failure ---------------------------------------------------------

#[tokio::test]
#[serial]
async fn mqtt_save_unwritable_dir_is_500() {
    cec::mock::reset();
    // Build state whose settings file lives under an unwritable directory.
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("ro");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("config.json"), "{}").unwrap();
    std::fs::set_permissions(&sub, std::os::unix::fs::PermissionsExt::from_mode(0o555)).unwrap();

    let settings = std::sync::Arc::new(
        capi::settings::Settings::load(&sub.join("config.json"))
            .unwrap()
            .0,
    );
    let state = {
        // Rebuild app state around these settings (mirror of fixture but with
        // our own Settings instance).
        let hub = Arc::new(EventHub::new(64));
        let logs = capi::LogRing::new(16);
        let bus = Arc::new(BusState::new());
        let metrics = Arc::new(capi::Metrics::default());
        let adapter = AdapterHandle::new();
        let registry = Arc::new(capi::strategies::Registry::new());
        let steward = Arc::new(Steward::spawn(
            bus.clone(),
            hub.clone(),
            settings.clone(),
            adapter.clone(),
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
        ));
        let mqtt = capi::mqtt::MqttHandle::new();
        capi::server::AppState::new(
            settings, hub, logs, bus, adapter, steward, registry, metrics, mqtt,
        )
    };

    let app = capi::server::build_router(state.clone());
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/settings/mqtt")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"broker":"tcp://x:1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);
    std::fs::set_permissions(&sub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

// -- exec volume forced-target invalid -----------------------------------------------

#[test]
fn volume_forced_target_invalid_is_err() {
    cec::mock::reset();
    let cfg = capi::cec::Configuration {
        device_name: "v".into(),
        device_type: capi::cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    };
    let conn = std::sync::Arc::new(capi::cec::Connection::open(&cfg).unwrap());
    conn.force_opened_for_test();
    let registry = capi::strategies::Registry::new();
    let bus = capi::BusState::new();
    let err = capi::exec::volume_action(
        &conn,
        &bus,
        &registry,
        capi::strategies::Action::VolumeUp,
        Some(99),
    );
    assert!(err.is_err());
}

// -- classify feature-abort without params falls through ------------------------------

#[test]
fn classify_feature_abort_no_params_falls_through() {
    use capi::strategies::{StratResult, StratStatus};
    use capi::types::BusFrameEntry;
    let mut res = StratResult {
        strategy: "t".into(),
        status: StratStatus::AckedNoReply,
        acked: true,
        reply_opcode: 0,
        reply_name: String::new(),
        abort_opcode: 0,
        elapsed_ms: 0,
        error: String::new(),
        steps: vec![],
    };
    let frame = BusFrameEntry {
        timestamp: chrono::Utc::now(),
        initiator: 5,
        destination: 4,
        opcode: "0x00".into(), // FEATURE_ABORT
        ack: true,
        eom: true,
        opcode_set: true,
        params_hex: vec![], // no params -> abort arm skipped
    };
    capi::strategies::classify_for_test(&mut res, &[frame], None, 4);
    assert_eq!(res.status, StratStatus::AckedNoReply);
}
