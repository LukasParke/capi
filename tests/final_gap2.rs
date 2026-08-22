//! Final-gap batch 2: bridge pre-session no-ops, percent-decode arms, exec
//! display, strategy reply mappings, mqtt parse, supervisor open-failure
//! backoff (feature = "mock-cec").

#![cfg(feature = "mock-cec")]

mod common;

use capi::cec::{self, LogicalAddress, Opcode};
use capi::mqtt;
use capi::{AdapterHandle, BusState};
use std::sync::Arc;

use serial_test::serial;

// -- bridge gates before registration -------------------------------------------

#[test]
#[serial]
fn bridges_silently_ignore_events_before_any_session() {
    // cb_param is NULL until the first Connection::open installs callbacks;
    // every emitter must be a silent no-op in that state.
    cec::mock::reset();
    capi::cec::mock::emit_log_detached(2, "too early");
    capi::cec::mock::emit_keypress_detached(1, 5);
    capi::cec::mock::emit_alert_detached(3, 0, 9);
    capi::cec::mock::emit_source_activated_detached(4, true);
    capi::cec::mock::emit_config_changed_detached();
    capi::cec::mock::emit_menu_detached(1);
    capi::cec::mock::emit_command_detached(0, 4, 0x8F);
}

#[test]
#[serial]
fn bridges_after_close_also_noop() {
    cec::mock::reset();
    let cfg = capi::cec::Configuration {
        device_name: "ac".into(),
        device_type: capi::cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    };
    let conn = Arc::new(cec::Connection::open(&cfg).unwrap());
    conn.close().unwrap(); // deregisters cb_param

    // All emitters must be silent no-ops post-close.
    capi::cec::mock::emit_log_detached(2, "post");
    capi::cec::mock::emit_command_detached(0, 4, 0x8F);
}

// -- server/mod units --------------------------------------------------------------

#[test]
fn api_map_exec_covers_all_variants() {
    use capi::exec::ExecError;
    use capi::server::api_map_exec_for_test;
    let cases = [
        ExecError::Other("boom".into()),
        ExecError::MissingKey,
        ExecError::InvalidLogicalAddress,
        ExecError::AdapterUnavailable,
    ];
    for e in cases {
        let resp = api_map_exec_for_test(&e);
        assert!(resp.status().as_u16() >= 400);
    }
}

#[test]
fn exec_error_display_arms() {
    use capi::exec::ExecError;
    assert_eq!(
        ExecError::AdapterUnavailable.to_string(),
        "CEC adapter not available"
    );
    assert_eq!(ExecError::InvalidKey.to_string(), "invalid key");
    assert_eq!(
        ExecError::MissingKey.to_string(),
        "either 'key' or 'keycode' must be provided (keycode 0 = select; use key:\"select\")"
    );
    assert_eq!(
        ExecError::InvalidLogicalAddress.to_string(),
        "invalid logical address"
    );
    assert_eq!(ExecError::InvalidHdmiPort.to_string(), "invalid HDMI port");
    assert_eq!(ExecError::Other("x".into()).to_string(), "x");
}

#[test]
fn strategy_expected_reply_extra_mappings() {
    use capi::strategies::{expected_reply_opcode_for_test, Step, StepKind};
    let step = |kind, op| Step {
        kind,
        target: LogicalAddress::UNKNOWN,
        key: capi::cec::Keycode(0),
        wait: false,
        hold_ms: 0,
        opcode: op,
        params: vec![],
        delay_ms: 0,
    };
    assert_eq!(
        expected_reply_opcode_for_test(&step(StepKind::Transmit, Opcode::GIVE_AUDIO_STATUS)),
        Some(Opcode::REPORT_AUDIO_STATUS)
    );
    assert_eq!(
        expected_reply_opcode_for_test(&step(StepKind::Transmit, Opcode::GET_CEC_VERSION)),
        Some(Opcode::CEC_VERSION)
    );
    assert_eq!(
        expected_reply_opcode_for_test(&step(StepKind::Transmit, Opcode::GIVE_OSD_NAME)),
        Some(Opcode::SET_OSD_NAME)
    );
    assert_eq!(
        expected_reply_opcode_for_test(&step(StepKind::SendUserControl, Opcode(0))),
        None
    );
}

// -- mqtt parse ---------------------------------------------------------------------

#[test]
fn mqtt_host_without_port_defaults_1883() {
    let h = capi::mqtt::MqttHandle::new();
    let cfg = capi::types::MqttConfig {
        broker: "localhost".into(),
        ..Default::default()
    };
    let (_, rx) = tokio::sync::mpsc::unbounded_channel();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();
    h.start(cfg, tokio::sync::broadcast::channel(8).1, rx_clone(rx));
    h.stop();
    assert!(!h.is_connected());
}

fn rx_clone(
    _r: tokio::sync::mpsc::UnboundedReceiver<mqtt::MqttCommand>,
) -> tokio::sync::mpsc::UnboundedSender<mqtt::MqttCommand> {
    tokio::sync::mpsc::unbounded_channel().0
}

// -- supervisor open-failure backoff -------------------------------------------------

#[test]
#[serial]
fn supervisor_open_failure_backs_off_then_shuts_down() {
    use capi::supervisor::SupervisorDeps;

    let dir = tempfile::tempdir().unwrap();
    let (settings, _) = capi::settings::Settings::load(&dir.path().join("config.json")).unwrap();
    let deps = SupervisorDeps {
        settings: Arc::new(settings),
        adapter: AdapterHandle::new(),
        bus: Arc::new(BusState::new()),
        hub: Arc::new(capi::events::EventHub::new(16)),
    };

    // Fail the first libcec_open; supervisor logs + backs off, then we shut down.
    capi::cec::mock::set_fail_next(1);
    let handle = std::thread::spawn(move || {
        capi::supervisor::run_supervisor(
            deps,
            "fail".into(),
            "/dev/mock0".into(),
            false,
            Arc::new(|_, _| {}),
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    capi::supervisor::SHUTDOWN_FLAG.store(true, std::sync::atomic::Ordering::SeqCst);
    handle.join().expect("clean exit");
    capi::supervisor::SHUTDOWN_FLAG.store(false, std::sync::atomic::Ordering::SeqCst);
}
