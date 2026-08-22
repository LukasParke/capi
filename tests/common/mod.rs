//! Shared fixtures for integration tests: a fully wired AppState whose
//! adapter is intentionally NOT connected (hardware-free), plus helper
//! assertions for the JSON envelope.

#![allow(dead_code)]

use capi::cec;
use capi::events::{EventHub, LogRing, Metrics};
use capi::mqtt::MqttHandle;
use capi::server::AppState;
use capi::settings::Settings;
use capi::steward::Steward;
use capi::strategies::Registry;
use capi::{AdapterHandle, BusState};
use std::sync::Arc;

pub fn app_state() -> AppState {
    let dir = tempfile_dir();
    let settings_path = dir.join("config.json");
    let (settings, _) = Settings::load(&settings_path).expect("settings load");
    let settings = Arc::new(settings);

    let hub = Arc::new(EventHub::new(64));
    let logs = LogRing::new(50);
    let bus = Arc::new(BusState::new());
    let metrics = Arc::new(Metrics::default());
    let adapter = AdapterHandle::new();
    let registry = Arc::new(Registry::new());
    let steward = Arc::new(Steward::spawn(
        bus.clone(),
        hub.clone(),
        settings.clone(),
        adapter.clone(),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    ));
    let mqtt = MqttHandle::new();

    AppState::new(
        settings, hub, logs, bus, adapter, steward, registry, metrics, mqtt,
    )
}

/// Unique temp dir per call (config.json lives next to the "binary").
pub fn tempfile_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "capi-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

pub fn envelope(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).expect("valid JSON envelope")
}

pub fn assert_success(v: &serde_json::Value) {
    assert_eq!(v["status"], "success", "expected success envelope: {v}");
}

/// All in-process libcec sessions serialize on this lock during tests:
/// libcec keeps global state that is not safe against concurrent
/// open/close churn from parallel test threads.
pub static CEC_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Guard helper: poison-tolerant lock acquisition.
pub fn cec_serial() -> std::sync::MutexGuard<'static, ()> {
    CEC_SERIAL.lock().unwrap_or_else(|p| p.into_inner())
}

/// One shared, opened, monitor-only session per test process. Tests clone the
/// Arc and never close it: repeated headless open/close churn trips a libcec
/// internal quirk under instrumentation, while a single long-lived session is
/// stable. The process exit reclaims everything (no Drop runs for statics).
#[cfg(feature = "mock-cec")]
pub fn shared_session() -> std::sync::Arc<capi::cec::Connection> {
    use std::sync::{Arc, LazyLock};
    static SHARED: LazyLock<Arc<capi::cec::Connection>> = LazyLock::new(|| {
        let cfg = capi::cec::Configuration {
            device_name: "itest-shared".into(),
            device_type: capi::cec::DeviceType::RECORDING,
            physical_address: 0xFFFF,
            base_device: capi::cec::LogicalAddress::TV,
            hdmi_port: 1,
            monitor_only: true,
            activate_source: false,
            wake_devices: vec![],
            power_off_devices: vec![],
        };
        let conn = Arc::new(capi::cec::Connection::open(&cfg).expect("headless session"));
        conn.force_opened_for_test();
        conn
    });
    SHARED.clone()
}

#[cfg(feature = "mock-cec")]
/// AppState fixture wired to the shared monitor session.
pub fn app_state_with_monitor_session() -> AppState {
    let state = app_state();
    state.adapter().set(Some(shared_session()));
    state
}

/// Bundles a live mock session with its app state for mock-suite tests.
pub struct StateBundle {
    pub state: AppState,
    pub conn: std::sync::Arc<capi::cec::Connection>,
    pub cfg: capi::cec::Configuration,
}

impl StateBundle {
    pub fn cfg_clone(&self) -> capi::cec::Configuration {
        self.cfg.clone()
    }
}

#[cfg(feature = "mock-cec")]
/// AppState wired to a LIVE (transmit-capable) mock session: dev endpoints
/// and command paths run their full success logic.
pub fn app_state_with_live_session() -> AppState {
    cec::mock::reset();
    let state = app_state();
    let cfg = capi::cec::Configuration {
        device_name: "live-itest".into(),
        device_type: capi::cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: capi::cec::LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    };
    let conn = std::sync::Arc::new(capi::cec::Connection::open(&cfg).expect("mock session"));
    conn.force_opened_for_test();
    state.bus().set_frame_ring_capacity(256);
    state.adapter().set(Some(conn.clone()));

    // Synchronous event sink: connection events feed the production dispatch
    // chain inline (no consumer-thread hop) so rings/classifiers are
    // deterministic in tests.
    {
        let bus = state.bus().clone();
        let hub = state.hub().clone();
        let steward = state.steward().clone();
        let logs = state.logs().clone();
        let conn_for_sink = conn.clone();
        conn.set_event_sink(Some(Arc::new(move |ev| {
            capi::dispatch::dispatch_cec_event(
                &conn_for_sink,
                &bus,
                &hub,
                &steward,
                &logs,
                ev.clone(),
            );
        })));
    }

    state
}
