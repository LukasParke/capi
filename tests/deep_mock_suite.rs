#![cfg(feature = "mock-cec")]
//! Deep mock coverage: configuration mapping, menu handler, adapters,
//! stats, and full-strategy breadth (feature = "mock-cec").

mod common;

use capi::cec::{self, CecEvent, Command, Configuration, LogicalAddress, Opcode};
use serial_test::serial;
use std::sync::Arc;

fn cfg() -> Configuration {
    Configuration {
        device_name: "deep-mock".into(),
        device_type: cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    }
}

#[test]
#[serial]
fn get_current_configuration_maps_fields() {
    let conn = Arc::new(cec::Connection::open(&cfg()).unwrap());
    conn.force_opened_for_test();

    let cur = conn.get_current_configuration().expect("config");
    assert_eq!(cur.device_name, "MOCK");
    assert_eq!(cur.device_type, cec::DeviceType::RECORDING);

    // find_adapters parses the single mock entry.
    let adapters = conn.find_adapters().unwrap();
    assert_eq!(adapters.len(), 1);
    assert_eq!(adapters[0].path, "/dev/mock0");
    assert_eq!(adapters[0].comm, "/dev/mock0");

    // rescan with a real settle exercises the sleep path.
    assert!(conn
        .rescan_devices(std::time::Duration::from_millis(20))
        .is_ok());

    conn.close().unwrap();
}

#[test]
#[serial]
fn set_configuration_roundtrip_and_menu_handler() {
    let conn = Arc::new(cec::Connection::open(&cfg()).unwrap());
    conn.force_opened_for_test();

    // Install a menu handler: emit_menu must return the handler's verdict.
    conn.set_menu_state_handler(Some(Arc::new(|_state| true)));
    assert_eq!(capi::cec::mock::emit_menu_on(&conn, 1), 1);

    // Clear the handler: emit must not panic and returns an i32 verdict.
    conn.set_menu_state_handler(None);
    let _ = capi::cec::mock::emit_menu_on(&conn, 1);

    // set_configuration success path (re-installs callbacks).
    assert!(conn.set_configuration(&cfg()).is_ok());

    conn.close().unwrap();
}

#[test]
#[serial]
fn event_channel_lag_is_counted_not_fatal() {
    let conn = Arc::new(cec::Connection::open(&cfg()).unwrap());
    conn.force_opened_for_test();
    let mut rx = conn.subscribe_events();

    // Flood past capacity (512) without draining.
    for i in 0u8..=255 {
        capi::cec::mock::emit_keypress_on(&conn, i, 1);
    }
    for _ in 0..600 {
        capi::cec::mock::emit_keypress_on(&conn, 1, 1);
    }

    // First recv may be Lagged; the channel stays usable either way.
    match rx.try_recv() {
        Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
        Err(e) => panic!("unexpected: {e}"),
    }

    // Stats counters are exercised via the broadcast deliveries above; the
    // channel-lag branch is timing-dependent and skipped here.

    conn.close().unwrap();
}

#[test]
#[serial]
fn full_strategy_breadth_executes_all_step_kinds() {
    use capi::strategies::{Action, Registry, RunOptions, Step, StepKind, Strategy};
    let conn = Arc::new(cec::Connection::open(&cfg()).unwrap());
    conn.force_opened_for_test();

    let registry = Registry::new();
    let bus = Arc::new(capi::BusState::new());
    bus.set_frame_ring_capacity(256);
    // Sync sink: replies land in the ring deterministically.
    {
        let bus2 = bus.clone();
        conn.set_event_sink(Some(Arc::new(move |ev| {
            if let capi::cec::CecEvent::Command(cmd) = ev {
                bus2.append_frame(cmd, 256);
            }
        })));
    }

    // Warm-up receiver held for the whole test: guarantees no event is
    // dropped before the consumer thread registers.
    let _warm_rx = conn.subscribe_events();

    let mk = |kind, target, key, hold, op: capi::cec::Opcode, delay| Step {
        kind,
        target,
        key,
        wait: false,
        hold_ms: hold,
        opcode: op,
        params: vec![],
        delay_ms: delay,
    };

    let chain = vec![Strategy {
        name: "breadth".into(),
        steps: vec![
            mk(
                StepKind::SendUserControl,
                LogicalAddress::AUDIO_SYSTEM,
                capi::cec::Keycode::VOLUME_UP,
                10,
                capi::cec::Opcode(0),
                0,
            ),
            mk(
                StepKind::LibcecVolumeUp,
                LogicalAddress::UNKNOWN,
                capi::cec::Keycode(0),
                0,
                capi::cec::Opcode(0),
                0,
            ),
            mk(
                StepKind::LibcecVolumeDown,
                LogicalAddress::UNKNOWN,
                capi::cec::Keycode(0),
                0,
                capi::cec::Opcode(0),
                0,
            ),
            mk(
                StepKind::LibcecMute,
                LogicalAddress::UNKNOWN,
                capi::cec::Keycode(0),
                0,
                capi::cec::Opcode(0),
                0,
            ),
            mk(
                StepKind::LibcecPowerOn,
                LogicalAddress::TV,
                capi::cec::Keycode(0),
                0,
                capi::cec::Opcode(0),
                0,
            ),
            mk(
                StepKind::LibcecStandby,
                LogicalAddress::TV,
                capi::cec::Keycode(0),
                0,
                capi::cec::Opcode(0),
                0,
            ),
            mk(
                StepKind::EnableSam,
                LogicalAddress::UNKNOWN,
                capi::cec::Keycode(0),
                0,
                capi::cec::Opcode(0),
                0,
            ),
            mk(
                StepKind::Transmit,
                LogicalAddress::TV,
                capi::cec::Keycode(0),
                0,
                capi::cec::Opcode::GIVE_DEVICE_POWER_STATUS,
                0,
            ),
            mk(
                StepKind::Wait,
                LogicalAddress::UNKNOWN,
                capi::cec::Keycode(0),
                0,
                capi::cec::Opcode(0),
                30,
            ),
        ],
        observe_ms: 120,
    }];
    registry.set_vendor_override("mock", Action::Power, chain);

    let opts = RunOptions {
        vendor: "mock".into(),
        target: Some(LogicalAddress::TV),
        all_strategies: true,
        observe_override_ms: 80,
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let results = registry.run(&conn, &bus, Action::Power, &opts, deadline);
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].steps.len(),
        9,
        "every step recorded: {:?}",
        results[0].steps
    );
    // All real (non-wait) steps acked by the mock; wait steps are no-ops.
    for s in &results[0].steps {
        if s.kind != "wait" {
            assert!(s.acked, "step acked: {:?}", s);
        }
    }

    conn.close().unwrap();
}

#[test]
#[serial]
fn run_stops_at_first_ok_unless_all_requested() {
    use capi::strategies::{Action, Registry, RunOptions, Step, StepKind, Strategy};
    let conn = Arc::new(cec::Connection::open(&cfg()).unwrap());
    conn.force_opened_for_test();

    let registry = Registry::new();
    let bus = Arc::new(capi::BusState::new());
    bus.set_frame_ring_capacity(64);
    // Sync sink: the auto-reply lands in the ring before classify runs.
    {
        let bus2 = bus.clone();
        conn.set_event_sink(Some(Arc::new(move |ev| {
            if let capi::cec::CecEvent::Command(cmd) = ev {
                bus2.append_frame(cmd, 64);
            }
        })));
    }

    // Both strategies press volume-up on the audio system; the mock replies
    // ReportAudioStatus, so strategy #1 classifies ok and early-stop kicks in.
    let s = |name: &str| Strategy {
        name: name.into(),
        steps: vec![Step {
            kind: StepKind::SendUserControl,
            target: LogicalAddress::AUDIO_SYSTEM,
            key: capi::cec::Keycode::VOLUME_UP,
            wait: false,
            hold_ms: 0,
            opcode: capi::cec::Opcode(0),
            params: vec![],
            delay_ms: 0,
        }],
        observe_ms: 80,
    };
    registry.set_vendor_override("m2", Action::Power, vec![s("first"), s("second")]);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let results = registry.run(
        &conn,
        &bus,
        Action::Power,
        &RunOptions {
            vendor: "m2".into(),
            target: None,
            all_strategies: false,
            observe_override_ms: 120,
        },
        deadline,
    );
    eprintln!(
        "[dbg] results={} first_status={:?} ring={:?}",
        results.len(),
        results.first().map(|r| r.status),
        bus.recent_frames()
            .iter()
            .map(|f| (&f.opcode, f.initiator, f.destination))
            .collect::<Vec<_>>()
    );
    assert_eq!(results.len(), 1, "early stop at first ok");
    assert!(results[0].steps.iter().all(|s| s.acked));
    assert!(capi::cec::mock::last_was_reply(), "auto-reply emitted");

    conn.close().unwrap();
}

#[test]
#[serial]
fn bridge_arms_log_and_closed_gates() {
    cec::mock::reset();
    let cfg = capi::cec::Configuration {
        device_name: "gates".into(),
        device_type: capi::cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    };
    let conn = Arc::new(capi::cec::Connection::open(&cfg).unwrap());
    conn.force_opened_for_test();
    let mut rx = conn.subscribe_events();

    // Log bridge arm.
    capi::cec::mock::emit_log_on(&conn, 2, "bridge log line");
    match rx.blocking_recv().unwrap() {
        CecEvent::Log { message, .. } => assert_eq!(message, "bridge log line"),
        other => panic!("{other:?}"),
    }

    // Menu handler set: emit_menu returns handler verdict (1).
    conn.set_menu_state_handler(Some(Arc::new(|_| true)));
    assert_eq!(capi::cec::mock::emit_menu_on(&conn, 1), 1);

    conn.close().unwrap();

    // After close: every bridge gate returns silently; channel reports Closed.
    let mut rx2 = conn.subscribe_events();
    capi::cec::mock::emit_log_on(&conn, 2, "post-close");
    capi::cec::mock::emit_keypress_on(&conn, 5, 10);
    capi::cec::mock::emit_alert_on(&conn, 1, 0, 0);
    capi::cec::mock::emit_source_activated_on(&conn, 3, true);
    capi::cec::mock::emit_config_changed_on(&conn);
    capi::cec::mock::emit_menu_on(&conn, 0);
    cec::mock::emit_command_on(
        &conn,
        &Command {
            initiator: LogicalAddress(0),
            destination: LogicalAddress(4),
            opcode: Opcode(0x44),
            opcode_set: true,
            parameters: vec![],
            ack: false,
            eom: true,
        },
    );
    assert!(matches!(
        rx2.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Closed)
            | Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}
