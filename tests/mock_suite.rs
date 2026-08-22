#![cfg(feature = "mock-cec")]
//! Mock-backend suite (`--features mock-cec`): drives SUCCESS paths of every
//! hardware-bound flow through the real callback/dispatch chain, with the
//! virtual adapter's deterministic responses.

use capi::cec;
mod common;

use capi::cec::{CecEvent, Command, LogicalAddress, Opcode};
use serial_test::serial;
use std::sync::Arc;

fn recv_within(
    rx: &mut tokio::sync::broadcast::Receiver<capi::cec::CecEvent>,
    deadline: std::time::Instant,
) -> Option<capi::cec::CecEvent> {
    loop {
        match rx.try_recv() {
            Ok(ev) => return Some(ev),
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => return None,
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {}
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn live_state() -> common::StateBundle {
    cec::mock::reset();
    let state = common::app_state();
    let cfg = cec::Configuration {
        device_name: "mock-suite".into(),
        device_type: cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false, // LIVE: transmits succeed against the mock
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    };
    let conn = Arc::new(cec::Connection::open(&cfg).expect("mock session"));
    conn.force_opened_for_test();
    state.adapter().set(Some(conn.clone()));
    state.bus().set_frame_ring_capacity(64);
    common::StateBundle { state, conn, cfg }
}

#[test]
#[serial]
fn connection_success_paths_answer_from_mock() {
    let bundle = live_state();
    let c = bundle.conn.clone();

    eprintln!("@open_adapter");
    eprintln!("@open_adapter");
    assert!(c.open_adapter("/dev/mock0").is_ok());
    assert!(c.get_lib_info().unwrap().contains("mock libcec"));
    eprintln!("@power_on");
    assert!(c.power_on(LogicalAddress::TV).is_ok());
    eprintln!("@standby");
    assert!(c.standby(LogicalAddress::TV).is_ok());
    assert_eq!(
        c.get_device_power_status(LogicalAddress::TV).unwrap(),
        capi::cec::PowerStatus::ON
    );
    eprintln!("@power_status");
    eprintln!("@vendor");
    assert_eq!(c.get_device_vendor_id(LogicalAddress(4)).unwrap(), 0x809819);
    assert_eq!(
        c.get_device_physical_address(LogicalAddress(4)).unwrap(),
        0x2000
    );
    eprintln!("@osd");
    assert_eq!(c.get_device_osd_name(LogicalAddress(4)).unwrap(), "MOCKBOX");
    assert_eq!(
        c.get_device_menu_language(LogicalAddress(4)).unwrap(),
        "eng"
    );
    assert_eq!(
        c.get_active_source().unwrap(),
        LogicalAddress::PLAYBACK_DEVICE_1
    );
    assert!(c.is_active_source(LogicalAddress(4)));
    assert!(c.poll_device(LogicalAddress(5)));
    eprintln!("@audio");
    let (_vol, _muted, _raw) = c.get_audio_status().unwrap();
    let (vol, muted, _raw) = c.get_audio_status().unwrap();
    assert_eq!((vol, muted), (37, false));
    assert!(c.volume_up(true).is_ok());
    assert!(c
        .send_keypress(LogicalAddress(0), capi::cec::Keycode::SELECT, true)
        .is_ok());

    eprintln!("@set_hdmi_port");
    assert!(c.set_hdmi_port(LogicalAddress::TV, 2).is_ok());
    eprintln!("@monitor");
    eprintln!("@monitor");
    assert!(c.switch_monitoring(true).is_ok());
    eprintln!("@rescan");
    eprintln!("@rescan");
    assert!(c.rescan_devices(std::time::Duration::ZERO).is_ok());
    let addrs = c.get_logical_addresses().unwrap();
    assert!(addrs.contains(&LogicalAddress(4)));
    eprintln!("@first_la");
    // Mock mask includes TV(0), playback(4), audio(5): first = TV.
    assert_eq!(c.first_logical_address(), Some(LogicalAddress::TV));
    eprintln!("@with_poll");
    let _ = c.logical_addresses_with_poll(true);
    eprintln!("@ping");
    assert!(c.ping_tv().is_ok());
    eprintln!("@find");
    let _ = c.find_adapters();
    eprintln!("@getcfg");
    eprintln!("@getcfg");
    assert!(c.get_current_configuration().is_ok());
    eprintln!("@setcfg");
    eprintln!("@setcfg");
    assert!(c.set_configuration(&bundle.cfg).is_ok());
}

#[test]
#[serial]
fn transmit_records_exact_wire_bytes() {
    let bundle = live_state();
    let c = bundle.conn.clone();
    cec::mock::reset();

    c.transmit(&Command {
        initiator: LogicalAddress(4),
        destination: LogicalAddress::TV,
        opcode: Opcode::GIVE_DEVICE_POWER_STATUS,
        opcode_set: true,
        parameters: vec![0x10, 0x02],
        ack: false,
        eom: true,
    })
    .expect("mock acks");

    let last = cec::mock::last_transmit();
    assert_eq!(last.initiator, 4);
    assert_eq!(last.dest, 0);
    assert_eq!(last.opcode, 0x8F);
    assert_eq!(last.params, vec![0x10, 0x02]);
}

#[test]
#[serial]
fn emitted_command_flows_through_real_dispatch_chain() {
    let bundle = live_state(); // live_state internally serializes via its own session
    let state = bundle.state.clone();
    let _bus = state.bus();

    // The mock's callback chain posts to the CONNECTION event channel;
    // app-level fan-out is exercised separately.
    let mut conn_rx = bundle.conn.subscribe_events();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);

    // Inject a bus frame via the mock's callback -> bridge -> connection.
    cec::mock::emit_command_on(
        &bundle.conn,
        &Command {
            initiator: LogicalAddress(0),
            destination: LogicalAddress(15),
            opcode: Opcode::REPORT_POWER_STATUS,
            opcode_set: true,
            parameters: vec![0x00],
            ack: true,
            eom: true,
        },
    );

    let ev = recv_within(&mut conn_rx, deadline).expect("command event within deadline");
    if let CecEvent::Command(cmd) = &ev {
        assert_eq!(cmd.initiator.0, 0);
        assert_eq!(cmd.opcode.0, 0x90);
        assert_eq!(cmd.parameters, vec![0x00]);
    } else {
        panic!("expected Command event, got {ev:?}");
    }

    // Key press injection through the REAL mock callback chain.
    cec::mock::emit_keypress_on(&bundle.conn, 11, 200);
    let ev = recv_within(&mut conn_rx, deadline).expect("key event within deadline");
    assert!(matches!(ev, CecEvent::KeyPress { key: 11, .. }), "{ev:?}");
}

#[tokio::test]
#[serial]
async fn steward_snapshot_contains_mock_devices() {
    let bundle = live_state();
    let state = bundle.state.clone();

    state
        .steward()
        .enqueue_wait(
            capi::steward::JobKind::Full,
            std::time::Duration::from_secs(20),
        )
        .await
        .expect("job");

    let snap = state.bus().copy_snapshot();
    let las: Vec<i64> = snap
        .devices
        .iter()
        .filter_map(|d| d["logical_address"].as_i64())
        .collect();
    assert!(las.contains(&0), "TV present: {las:?}");
    assert!(las.contains(&4), "playback present");
    assert!(las.contains(&5), "audio present");
    let tv = snap
        .devices
        .iter()
        .find(|d| d["logical_address"] == 0)
        .unwrap();
    assert_eq!(tv["osd_name"], "MOCKBOX");
    assert_eq!(tv["power_status"], "On");
    assert_eq!(tv["vendor_id"], "0x809819");
    assert_eq!(tv["discovery"], "active");
}

#[tokio::test]
#[serial]
async fn api_success_paths_with_live_mock() {
    use tower::ServiceExt;
    let bundle = live_state();
    let app = capi::server::build_router(bundle.state.clone());

    async fn get_json(app: App, uri: &str) -> serde_json::Value {
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    type App = axum::Router;
    use axum::body::Body;

    let v = get_json(app.clone(), "/api/power/status").await;
    assert_eq!(v["data"]["status"], "On");

    let v = get_json(app.clone(), "/api/audio/status").await;
    assert_eq!(v["data"]["volume"], 37);

    let v = get_json(app, "/api/source/active").await;
    assert_eq!(v["data"]["active_source"], 4);
}
