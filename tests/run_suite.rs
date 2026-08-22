//! In-process boot/shutdown wiring coverage for `server::run`.

#![cfg(feature = "mock-cec")]

use capi::server;
use capi::settings;
use serial_test::serial;

fn base_flags(bind: String) -> settings::Flags {
    settings::Flags {
        bind,
        name: "run-suite".into(),
        adapter: "/dev/mock0".into(),
        token: String::new(),
        mqtt_broker: String::new(),
        mqtt_user: String::new(),
        mqtt_pass: String::new(),
        mqtt_prefix: "capi-run".into(),
        cec_monitor: false,
        shutdown_after_ms: None,
        config_dir: None,
        do_update: false,
        show_version: false,
    }
}

#[tokio::test]
#[serial]
async fn run_boots_serves_and_shuts_down_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap(); // config.json lands here

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener); // free the port for the server

    let flags = settings::Flags {
        shutdown_after_ms: Some(700),
        token: "run-token".into(),
        ..base_flags(format!("127.0.0.1:{port}"))
    };

    // Drive run() concurrently; it returns 0 on clean graceful shutdown.
    let handle = tokio::spawn(async move { server::run(flags).await });

    let client = reqwest::Client::new();

    // Poll /api/health until live (open endpoint).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Ok(resp) = client
            .get(format!("http://127.0.0.1:{port}/api/health"))
            .send()
            .await
        {
            assert_eq!(resp.status(), 200);
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "server never came up"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let code = tokio::time::timeout(std::time::Duration::from_secs(15), handle)
        .await
        .expect("shutdown within 15s")
        .expect("join");
    assert_eq!(code, 0);
}

#[tokio::test]
#[serial]
async fn run_bind_conflict_returns_one() {
    // Occupy a port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let flags = base_flags(format!("127.0.0.1:{port}"));
    let dir = tempfile::tempdir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let code = server::run(flags).await;
    assert_eq!(code, 1);
    let _ = listener;
}
