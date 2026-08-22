//! Bind-failure branch: second instance on an occupied port exits 1.

#![cfg(feature = "mock-cec")]

use serial_test::serial;
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
#[serial]
fn bind_conflict_exits_nonzero() {
    let dir = tempfile::tempdir().unwrap();
    let bin = env!("CARGO_BIN_EXE_capi");

    // Occupier: a healthy instance on a random port.
    let port = 18800 + (std::process::id() % 500) as u16;
    let mut first = Command::new(bin)
        .args(["-bind", &format!("127.0.0.1:{port}"), "-token", "x"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(800));

    // Second instance on the same port must exit non-zero quickly.
    let out = Command::new(bin)
        .env("CAPI_CONFIG_DIR_FOR_TEST", dir.path())
        .args(["-bind", &format!("127.0.0.1:{port}")])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "expected exit 1");
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot bind"));

    let _ = first.kill();
    let _ = first.wait();
}
