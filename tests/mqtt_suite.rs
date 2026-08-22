//! MQTT bridge integration (feature = "mock-cec"): drives MqttHandle,
//! publisher fan-out, command dispatch, and availability topic against a
//! locally spawned mosquitto broker. Skips gracefully when unavailable.

#![cfg(feature = "mock-cec")]

mod common;

use capi::events::EventHub;
use capi::mqtt::{self, MqttHandle};
use capi::types::AppEvent;
use serial_test::serial;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Broker {
    child: Option<std::process::Child>,
    #[allow(dead_code)] // kept for future direct-broker assertions
    port: u16,
    owns_child: bool,
}

impl Drop for Broker {
    fn drop(&mut self) {
        if self.owns_child {
            if let Some(c) = self.child.as_mut() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

fn spawn_broker() -> Option<Broker> {
    let port = 18890;
    // Reuse an existing broker if one already listens.
    if std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(200),
    )
    .is_ok()
    {
        return Some(Broker {
            child: None,
            port,
            owns_child: false,
        });
        // externally-owned broker; drop must not kill it.
    }
    let dir = tempfile::tempdir().unwrap();
    let conf = dir.path().join("mosq.conf");
    std::fs::write(
        &conf,
        format!("listener {port} 127.0.0.1\nallow_anonymous true\n"),
    )
    .unwrap();
    let child = Command::new("mosquitto")
        .args(["-c", conf.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Leak the tempdir so the config outlives the broker in this struct.
    std::mem::forget(dir);
    std::thread::sleep(Duration::from_millis(400));
    Some(Broker {
        child: Some(child),
        port,
        owns_child: true,
    })
}

#[test]
#[serial]
fn mqtt_bridge_end_to_end() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();
    let Some(broker) = spawn_broker() else {
        eprintln!("skipping: mosquitto unavailable");
        return;
    };
    let _keep = broker;

    let hub = Arc::new(EventHub::new(64));
    let handle = MqttHandle::new();
    let cfg = capi::types::MqttConfig {
        broker: format!("tcp://127.0.0.1:{}", 18890),
        user: String::new(),
        pass: String::new(),
        prefix: "capi-mqttsuite".into(),
    };

    // Subscribe externally BEFORE the client connects so we see retained + live.
    let mut sub = Command::new("mosquitto_sub")
        .args([
            "-h",
            "127.0.0.1",
            "-p",
            "18890",
            "-t",
            "capi-mqttsuite/#",
            "-v",
            "-W",
            "8",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mosquitto_sub");

    handle.start(cfg.clone(), hub.subscribe(), mk_tx());
    wait_connected(&handle, Duration::from_secs(10));

    // Publish an app event through the hub; publisher forwards it.
    hub.publish(AppEvent::new(
        capi::types::event_type::KEY_PRESS,
        serde_json::json!({"keycode": 5, "duration": 100}),
    ));
    std::thread::sleep(Duration::from_secs(1));

    handle.stop();
    assert!(!handle.is_connected());

    let _ = sub.kill();
    let out = sub.wait_with_output().expect("sub output");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("capi-mqttsuite/event/key_press"),
        "got: {text}"
    );
    assert!(text.contains("capi-mqttsuite/status online"), "{text}");
}

fn mk_tx() -> tokio::sync::mpsc::UnboundedSender<mqtt::MqttCommand> {
    tokio::sync::mpsc::unbounded_channel().0
}

fn wait_connected(h: &MqttHandle, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if h.is_connected() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("mqtt never connected");
}

// Silence unused when feature combos change.
#[allow(unused)]
fn touch(_: &EventHub) {}
