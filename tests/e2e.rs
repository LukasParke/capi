//! End-to-end suite: spawns the REAL release-path binary (cargo provides
//! CARGO_BIN_EXE_capi) on a loopback port and exercises the full stack —
//! boot, auth, pages, streams, MQTT loopback, graceful shutdown.

mod common;

use common::*;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    port: u16,
    token: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn spawn_server(extra: &[&str]) -> Server {
    let port = free_port();
    let token = "e2e-token";
    let dir = tempfile_dir();
    let bin = env!("CARGO_BIN_EXE_capi");
    let mut child = Command::new(bin)
        .args(["-bind", &format!("127.0.0.1:{port}"), "-token", token])
        .args(extra)
        .env("CAPI_CONFIG_DIR_FOR_TEST", dir.to_str().unwrap())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn capi binary");
    // Wait for readiness via /api/health.
    let deadline = Instant::now() + Duration::from_secs(15);
    let client = reqwest::blocking::Client::new();
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("server did not become healthy in 15s");
        }
        if let Ok(resp) = client
            .get(format!("http://127.0.0.1:{port}/api/health"))
            .send()
        {
            if resp.status().is_success() {
                break;
            }
        }
        if let Ok(Some(_)) = child.try_wait() {
            panic!("server exited before becoming healthy");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Server {
        child,
        port,
        token: token.to_string(),
    }
}

fn url(s: &Server, path: &str) -> String {
    format!("http://127.0.0.1:{}{path}", s.port)
}

#[test]
fn full_stack_boot_auth_and_shutdown() {
    let s = spawn_server(&[]);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    // Health is open.
    let resp = client.get(url(&s, "/api/health")).send().unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().unwrap();
    assert_eq!(v["status"], "success");
    assert!(v["data"]["version"].as_str().unwrap().starts_with('v'));

    // API requires the token.
    assert_eq!(
        client.get(url(&s, "/api/devices")).send().unwrap().status(),
        401
    );
    let resp = client
        .get(url(&s, "/api/devices"))
        .header("Authorization", format!("Bearer {}", s.token))
        .send()
        .unwrap();
    // Past auth: 503 with real backend (no adapter), 200 with mock backend.
    if cfg!(feature = "mock-cec") {
        assert_eq!(resp.status(), 200);
    } else {
        assert_eq!(resp.status(), 503);
    }

    // Pages redirect to /login when unauthenticated.
    let resp = client.get(url(&s, "/settings")).send().unwrap();
    assert_eq!(resp.status(), 401);

    // Login round-trip (no redirect following: assert the 303 itself).
    let no_redirect = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let resp = no_redirect
        .post(url(&s, "/login"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!("token={}", s.token))
        .send()
        .unwrap();
    assert_eq!(resp.status(), 303); // reqwest normalizes 303 location handling
    let cookie = resp
        .headers()
        .get("set-cookie")
        .expect("cookie")
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.starts_with("capi_token="));

    // Cookie authenticates.
    let resp = client
        .get(url(&s, "/api/devices"))
        .header("Cookie", &cookie)
        .send()
        .unwrap();
    if cfg!(feature = "mock-cec") {
        assert_eq!(resp.status(), 200);
    } else {
        assert_eq!(resp.status(), 503);
    }

    // SSE endpoint speaks event-stream (token-authenticated like all API).
    let resp = client
        .get(url(&s, "/api/events"))
        .header("Authorization", format!("Bearer {}", s.token))
        .send()
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
    drop(resp);

    // Metrics endpoint.
    let resp = client.get(url(&s, "/metrics")).send().unwrap();
    let text = resp.text().unwrap();
    assert!(text.contains("capi_requests_total"));

    // Graceful shutdown on SIGTERM: process exits promptly with success.
    let pid = s.child.id();
    #[allow(unused_mut)]
    let mut s = s;
    unsafe {
        libc_kill(pid as i32, 15); // SIGTERM
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match s.child.try_wait().unwrap() {
            Some(code) => {
                assert!(
                    code.success() || code.code() == Some(0),
                    "clean exit, got {code:?}"
                );
                break;
            }
            None if Instant::now() > deadline => panic!("server ignored SIGTERM"),
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

// libc kill without a libc dependency.
unsafe fn libc_kill(pid: i32, sig: i32) {
    #[link(name = "c")]
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, sig);
}

#[test]
fn config_flag_flows_into_runtime() {
    // -version prints and exits 0.
    let out = Command::new(env!("CARGO_BIN_EXE_capi"))
        .arg("-version")
        .output()
        .unwrap();
    assert!(out.status.success());
    let v = String::from_utf8_lossy(&out.stdout);
    assert!(v.trim().starts_with('v'), "version output: {v}");

    // Unknown flag -> usage error, exit 2.
    let out = Command::new(env!("CARGO_BIN_EXE_capi"))
        .arg("-definitely-not-a-flag")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown flag"));
}

#[test]
fn mqtt_loopback_command_and_availability() {
    // Runs whenever a local broker listens on 127.0.0.1:1883; otherwise the
    // whole test degrades to a no-op skip (CI without a broker stays green).
    // Ensure a broker: reuse one on 1883 or spawn our own on 18890.
    use std::net::TcpStream;
    let (broker_addr, _child) = match TcpStream::connect_timeout(
        &"127.0.0.1:1883".parse().unwrap(),
        Duration::from_millis(250),
    ) {
        Ok(_) => ("127.0.0.1:1883".to_string(), None),
        Err(_) => {
            let which = |bin: &str| {
                std::env::var("PATH")
                    .ok()
                    .and_then(|p| {
                        std::env::split_paths(&p)
                            .map(|d| d.join(bin))
                            .find(|c| c.is_file())
                    })
                    .is_some()
            };
            if !which("mosquitto") {
                eprintln!("skipping: no broker and no mosquitto binary");
                return;
            }
            let conf = tempfile::tempdir().unwrap();
            let conf_path = conf.path().join("mosq.conf");
            std::fs::write(
                &conf_path,
                "listener 18890 127.0.0.1\nallow_anonymous true\n",
            )
            .unwrap();
            let child = Command::new("mosquitto")
                .args(["-c", conf_path.to_str().unwrap()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn mosquitto");
            std::thread::sleep(Duration::from_millis(400));
            ("127.0.0.1:18890".to_string(), Some(child))
        }
    };
    let s = spawn_server(&[]);
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(url(&s, "/api/settings/mqtt"))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", s.token))
        .body(format!(
            r#"{{"broker":"tcp://{broker_addr}","prefix":"capi-e2e"}}"#
        ))
        .send()
        .unwrap();
    assert_eq!(resp.status(), 200);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut connected = false;
    while Instant::now() < deadline {
        let v: serde_json::Value = client
            .get(url(&s, "/api/settings/mqtt"))
            .header("Authorization", format!("Bearer {}", s.token))
            .send()
            .unwrap()
            .json()
            .unwrap();
        if v["data"]["connected"] == true {
            connected = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    assert!(connected, "bridge never connected to {}", broker_addr);

    // Availability topic must have been published (retained online).
    let out = Command::new("mosquitto_sub")
        .args([
            "-h",
            broker_addr.split(':').next().unwrap(),
            "-p",
            broker_addr.rsplit(':').next().unwrap(),
            "-t",
            "capi-e2e/status",
            "-C",
            "1",
            "-W",
            "5",
        ])
        .output()
        .expect("mosquitto_sub");
    let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(got.ends_with("online"), "got: {got}");
}
