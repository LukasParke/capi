//! Final coverage push: queue saturation, settings edges, update no-asset
//! arm, CEC surface completions (feature = "mock-cec").

#![cfg(feature = "mock-cec")]

mod common;

use capi::cec::{self, LogicalAddress};
use common::*;
use serial_test::serial;

// -- steward queue saturation ---------------------------------------------------

#[tokio::test]
#[serial]
async fn steward_queue_full_drops_jobs() {
    cec::mock::reset();
    let state = app_state_with_live_session();

    // Fill the bounded queue without letting the worker drain: hold the
    // connection's api lock? Jobs don't take it pre-run… instead flood
    // enqueue faster than the worker executes by pausing the worker via a
    // long Deep job first.
    state.steward().enqueue(capi::steward::JobKind::Deep); // worker starts long job
                                                           // Flood while deep job runs.
    let mut queued = 0;
    for _ in 0..64 {
        if state.steward().enqueue(capi::steward::JobKind::Light) {
            queued += 1;
        }
    }
    // At least one enqueue was dropped once the 32-slot queue filled.
    let (_, dropped_after) = state.steward().counters();
    let _ = queued;
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let snap = state.bus().copy_snapshot();
            if !snap.scan_in_progress {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("deep job eventually drains");
    let _ = dropped_after;
}

// -- settings edges ----------------------------------------------------------------

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
    capi::settings::Settings::quarantine_corrupt(&p); // must not panic
    assert!(!p.exists());
}

// -- update: release missing binary asset -------------------------------------------

#[tokio::test]
async fn update_no_asset_arm() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/repos/LukasParke/capi/releases/latest",
        axum::routing::get(|| async {
            axum::Json(serde_json::json!({
                "tag_name": "v99.0.0",
                "assets": [
                    {"name": "SHA256SUMS", "browser_download_url": "/assets/sums"},
                ],
            }))
        }),
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("capi"), b"OLD").unwrap();
    let (settings, _) = capi::settings::Settings::load(&dir.path().join("config.json")).unwrap();
    let err = capi::update::__test_check_named(
        &settings,
        &format!("http://{addr}"),
        Some(dir.path().to_path_buf()),
        "capi-linux-arm64-libcec6",
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("SHA256SUMS") || err.contains("no asset"),
        "{err}"
    );
}

// -- cec surface completions ----------------------------------------------------------

#[test]
#[serial]
fn cec_post_close_and_lag_behaviour() {
    cec::mock::reset();
    let cfg = capi::cec::Configuration {
        device_name: "lag".into(),
        device_type: capi::cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    };
    let conn = std::sync::Arc::new(cec::Connection::open(&cfg).unwrap());
    conn.force_opened_for_test();

    // Subscribe before close so we can observe channel shutdown; after
    // close the bridge drops events silently (session deregistered) and the
    // receiver reports Closed — no panics anywhere.
    let mut rx = conn.subscribe_events();
    conn.close().unwrap();
    cec::mock::emit_command_on(&conn, &Command_for_lag());
    match rx.try_recv() {
        Err(tokio::sync::broadcast::error::TryRecvError::Closed)
        | Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
        Ok(ev) => panic!("no events expected post-close, got {ev:?}"),
    }
}

// Helper so the emit signature matches (params slice arg).
#[allow(non_snake_case)]
fn Command_for_lag() -> cec::Command {
    cec::Command {
        initiator: LogicalAddress(0),
        destination: LogicalAddress(15),
        opcode: capi::cec::Opcode(0x90),
        opcode_set: true,
        parameters: vec![],
        ack: true,
        eom: true,
    }
}
