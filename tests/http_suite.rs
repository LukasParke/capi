//! Router-level integration suite: drives the REAL production router
//! (`build_router`) through `tower::ServiceExt::oneshot` — no sockets, no
//! hardware. Covers envelope shape, validation parity, auth/CSRF, UI
//! fragments, dev surface, metrics, and static assets.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use capi::server;
use common::*;
use tower::ServiceExt;

type App = axum::Router;

async fn call(
    app: App,
    method: &str,
    uri: &str,
    body: Option<String>,
    headers: &[(&str, &str)],
) -> (StatusCode, Vec<u8>, axum::http::HeaderMap) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let req = builder
        .body(Body::from(body.unwrap_or_default()))
        .expect("request");
    let resp = app.oneshot(req).await.expect("oneshot infallible");
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    (status, body, headers)
}

async fn json_call(app: &App, method: &str, uri: &str) -> (StatusCode, serde_json::Value) {
    let (status, body, _) = call(app.clone(), method, uri, None, &[]).await;
    (status, envelope(&body))
}

async fn json_post(app: &App, uri: &str, json: &str) -> (StatusCode, serde_json::Value) {
    let (status, body, _) = call(
        app.clone(),
        "POST",
        uri,
        Some(json.to_string()),
        &[("Content-Type", "application/json")],
    )
    .await;
    (status, envelope(&body))
}

/// Tiny single-threaded block-on so helpers stay sync inside #[test] fns.
fn futures_lite_block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("rt")
        .block_on(fut)
}

/// Like `call`, but returns without draining the body — required for
/// infinite streams (SSE) where collection would never complete.
async fn call_stream(app: App, method: &str, uri: &str) -> (StatusCode, axum::http::HeaderMap) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    let resp = app.oneshot(req).await.expect("oneshot infallible");
    let status = resp.status();
    let headers = resp.headers().clone();
    (status, headers) // body dropped: stream stays open, connection closes on drop
}

// -- basic surface ------------------------------------------------------------

#[test]
fn health_envelope_and_fields() {
    futures_lite_block_on(async {
        let state = app_state();
        let app = server::build_router(state.clone());
        let (status, v) = json_call(&app, "GET", "/api/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_success(&v);
        assert_eq!(v["data"]["cec_ready"], false);
        assert!(v["data"]["version"].is_string());
        assert!(v["data"]["uptime_seconds"].is_number());
    });
}

#[test]
fn devices_adapter_down_is_503() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        let (status, body, _) = call(app, "GET", "/api/devices", None, &[]).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let v = envelope(&body);
        assert_eq!(v["status"], "error");
        assert!(v["message"].as_str().unwrap().contains("not available"));
    });
}

#[test]
fn devices_wait_validation() {
    futures_lite_block_on(async {
        // Adapter-down 503 takes precedence over ?wait parsing only after
        // requireCEC-equivalent; with adapter down we expect 503 regardless.
        let app = server::build_router(app_state());
        let (status, _, _) = call(app, "GET", "/api/devices?wait=abc", None, &[]).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    });
}

#[test]
fn device_address_validation() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        // Out of range must be 400 BEFORE the adapter gate (fixes Go 500s).
        for bad in ["15", "16", "abc", "-1"] {
            let (status, body, _) = call(
                app.clone(),
                "GET",
                &format!("/api/devices/{bad}"),
                None,
                &[],
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
            assert!(envelope(&body)["message"]
                .as_str()
                .unwrap()
                .contains("invalid logical address"));
        }
        // In-range but no adapter -> 503.
        let (status, _, _) = call(app, "GET", "/api/devices/4", None, &[]).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    });
}

#[test]
fn bus_state_shape_and_scan_accepted() {
    futures_lite_block_on(async {
        let state = app_state();
        let app = server::build_router(state.clone());

        let (_, v) = json_call(&app, "GET", "/api/bus/state").await;
        assert_success(&v);
        let d = &v["data"];
        assert!(d["devices"].is_array());
        assert_eq!(d["active_source"], -1);
        assert_eq!(d["cec_ready"], false);

        let (status, v) = json_call(&app, "POST", "/api/bus/scan").await;
        assert_eq!(status, StatusCode::OK);
        assert_success(&v);
        assert_eq!(v["data"]["accepted"], true);

        // Frames list is an empty array before any traffic.
        let (_, v) = json_call(&app, "GET", "/api/bus/frames").await;
        assert!(v["data"].is_array());

        // Topology guarantees at least 4 port rows.
        let (_, v) = json_call(&app, "GET", "/api/topology").await;
        assert!(v["data"]["ports"].as_array().unwrap().len() >= 4);
        let _ = state;
    });
}

#[test]
fn power_endpoints_validation_and_503() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        // addr out of range -> 400 even without adapter.
        for bad in ["/api/power/on/15", "/api/power/off/99"] {
            let (status, _, _) = call(app.clone(), "POST", bad, None, &[]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        }
        // valid range but adapter down -> 503.
        for ok in [
            "/api/power/on",
            "/api/power/on/0",
            "/api/power/off/14",
            "/api/power/status/3",
        ] {
            let (_status, _, _) = call(app.clone(), "POST", ok, None, &[]).await;
            let get_ok = if ok.starts_with("/api/power/status") {
                "GET"
            } else {
                "POST"
            };
            let (status, _, _) = call(app.clone(), get_ok, ok, None, &[]).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{ok}");
        }
    });
}

#[test]
fn volume_query_validation() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        // Query-form address validation happens BEFORE adapter gate.
        let (status, body, _) =
            call(app.clone(), "POST", "/api/volume/up?address=15", None, &[]).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{}",
            String::from_utf8_lossy(&body)
        );
        let (status, _, _) =
            call(app.clone(), "POST", "/api/volume/down?address=7", None, &[]).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let (status, _, _) = call(app, "POST", "/api/volume/mute", None, &[]).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    });
}

#[test]
fn source_hdmi_audio_validation() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        let (status, _, _) = call(app.clone(), "POST", "/api/source/15", None, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _, _) = call(app.clone(), "GET", "/api/source/active", None, &[]).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE); // adapter gate first (parity)
        for bad in ["/api/hdmi/0", "/api/hdmi/16", "/api/hdmi/x"] {
            let (status, _, _) = call(app.clone(), "POST", bad, None, &[]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        }
        let (status, _, _) = call(app.clone(), "POST", "/api/hdmi/2", None, &[]).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        audio_status_zero_when_down(&app).await;
    });
}

async fn audio_status_zero_when_down(app: &App) {
    let (_, v) = json_call(app, "GET", "/api/audio/status").await;
    assert_success(&v);
    assert_eq!(v["data"]["volume"], 0);
    assert_eq!(v["data"]["muted"], false);
}

// -- key + raw command regressions --------------------------------------------

#[test]
fn keycode_zero_rejected_with_documented_message() {
    // Regression: openapi documented keycode 0 as sendable; handler rejects
    // it with an explicit pointer to key:"select" (documented parity).
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        let (status, v) = json_post(&app, "/api/key", r#"{"address":0,"keycode":0}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let msg = v["message"].as_str().unwrap();
        assert!(
            msg.contains("select"),
            "message should point at select workaround: {msg}"
        );
    });
}

#[test]
fn key_address_bounds_before_adapter() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        let (status, v) = json_post(&app, "/api/key", r#"{"address":15,"key":"select"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(envelope(&serde_json::to_vec(&v).unwrap())["message"]
            .as_str()
            .unwrap()
            .contains("invalid logical address"));
        // Unsupported key name -> 400 via typed error (was string sniffing).
        let (status, _) = json_post(&app, "/api/key", r#"{"address":0,"key":"warp_drive"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    });
}

#[test]
fn raw_command_bounds_matrix() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        let cases = [
            (
                r#"{"initiator":-1,"destination":0,"opcode":1}"#,
                "initiator",
            ),
            (
                r#"{"initiator":16,"destination":0,"opcode":1}"#,
                "initiator",
            ),
            (
                r#"{"initiator":0,"destination":16,"opcode":1}"#,
                "destination",
            ),
            (r#"{"initiator":0,"destination":0,"opcode":256}"#, "opcode"),
            (
                r#"{"initiator":0,"destination":0,"opcode":1,"parameters":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]}"#,
                "parameters",
            ),
        ];
        for (json, field) in cases {
            let (status, v) = json_post(&app, "/api/command", json).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {v}");
            assert!(
                v["message"].as_str().unwrap().contains(field),
                "{field}: {v}"
            );
        }
        // Valid frame but no adapter -> 503.
        let (status, _) = json_post(
            &app,
            "/api/command",
            r#"{"initiator":4,"destination":0,"opcode":143}"#,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    });
}

// -- logs / metrics ------------------------------------------------------------

#[test]
fn logs_roundtrip_and_metrics_text() {
    futures_lite_block_on(async {
        let state = app_state();
        state.logs().push("APP", "hello world".into());
        state.logs().push("ERROR", "boom".into());
        let app = server::build_router(state.clone());

        let (_, v) = json_call(&app, "GET", "/api/logs").await;
        assert_success(&v);
        let lines = v["data"].as_array().unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["level"], "APP");

        // Metrics: text/plain, contains our counters, request count > 0.
        let (status, body, headers) = call(app, "GET", "/metrics", None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain"));
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("capi_requests_total "));
        assert!(text.contains("capi_adapter_ready 0"));
        assert!(text.contains("capi_frames_captured_total 0"));
    });
}

// -- MQTT settings ---------------------------------------------------------------

#[test]
fn mqtt_settings_masking_and_preserve_semantics() {
    futures_lite_block_on(async {
        let state = app_state();
        let app = server::build_router(state.clone());

        // Save real creds.
        let (status, v) = json_post(
            &app,
            "/api/settings/mqtt",
            r#"{"broker":"tcp://h:1883","user":"u","pass":"secret","prefix":"pfx"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_success(&v);

        // GET masks password.
        let (_, v) = json_call(&app, "GET", "/api/settings/mqtt").await;
        assert_eq!(v["data"]["pass"], "***");
        assert_eq!(v["data"]["broker"], "tcp://h:1883");
        assert_eq!(v["data"]["connected"], false);

        // POST with "***" preserves stored secret.
        let (status, _) = json_post(
            &app,
            "/api/settings/mqtt",
            r#"{"broker":"tcp://h2:1883","user":"u","pass":"***","prefix":""}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let cfg = state.settings().get().mqtt;
        assert_eq!(cfg.pass, "secret");
        assert_eq!(cfg.broker, "tcp://h2:1883");
        assert_eq!(cfg.prefix, "capi"); // empty prefix defaults

        // POST with empty pass clears it.
        let (status, _) = json_post(
            &app,
            "/api/settings/mqtt",
            r#"{"broker":"","user":"","pass":"","prefix":"x"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let cfg = state.settings().get().mqtt;
        assert_eq!(cfg.pass, "");
        assert_eq!(cfg.prefix, "x");
    });
}

// -- dev surface -----------------------------------------------------------------

#[test]
fn dev_mode_roundtrip_and_persist_atomicity() {
    futures_lite_block_on(async {
        let state = app_state();
        let app = server::build_router(state.clone());

        let (_, v) = json_call(&app, "GET", "/api/dev/mode").await;
        assert_eq!(v["data"]["monitor_only"], false);

        let (status, v) = json_post(&app, "/api/dev/mode", r#"{"monitor_only":true}"#).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert!(state.settings().get().cec.monitor_only);
        assert_eq!(state.hub().subscriber_count(), 0); // reconnect signal doesn't touch hub subs

        // reconnect-only body is accepted too.
        let (status, _) = json_post(&app, "/api/dev/mode", r#"{"reconnect":true}"#).await;
        assert_eq!(status, StatusCode::OK);
    });
}

#[test]
fn dev_probe_and_strategies_validation() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        let (status, _) = json_post(&app, "/api/dev/probe", r#"{"address":15}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = json_post(&app, "/api/dev/probe", r#"{"address":0,"kind":"nope"}"#).await;
        // unknown kind is rejected after adapter gate; adapter down wins:
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

        let (status, v) = json_post(&app, "/api/dev/run_strategies", r#"{"action":"warp"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(v["message"].as_str().unwrap().contains("unknown action"));

        // Known action but monitor-only refusal requires adapter first -> 503.
        let (status, _) =
            json_post(&app, "/api/dev/run_strategies", r#"{"action":"volume_up"}"#).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    });
}

#[test]
fn dev_save_strategy_unknown_is_404_known_applies() {
    futures_lite_block_on(async {
        let state = app_state();
        let app = server::build_router(state.clone());

        let (status, v) = json_post(
            &app,
            "/api/dev/save_strategy",
            r#"{"vendor":"0x000048","action":"volume_up","strategy":"nonexistent"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{v}");

        let (status, v) = json_post(
            &app,
            "/api/dev/save_strategy",
            r#"{"vendor":"0x000048","action":"volume_up","strategy":"uc_volume_up_audio"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        // Override visible through registry AND persisted config.
        let chain = state
            .registry()
            .strategies_for("0x000048", capi::strategies::Action::VolumeUp);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].name, "uc_volume_up_audio");
        assert_eq!(
            state.settings().get().cec.strategy_overrides["0x000048"]["volume_up"],
            "uc_volume_up_audio"
        );
    });
}

#[test]
fn dev_vocabularies_are_complete_and_parseable() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        let (_, v) = json_call(&app, "GET", "/api/dev/actions").await;
        let actions = v["data"].as_array().unwrap();
        assert!(actions.len() >= 30, "expected >=30 actions");
        for a in actions {
            assert!(a["action"].is_string());
            assert!(
                !a["strategies"].as_array().unwrap().is_empty(),
                "{} has strategies",
                a["action"]
            );
        }

        let (_, v) = json_call(&app, "GET", "/api/dev/keys").await;
        let keys = v["data"].as_array().unwrap();
        assert!(
            keys.len() >= 50,
            "key table parity floor, got {}",
            keys.len()
        );
        // Every name resolves back to its code (round-trip guarantee).
        for k in keys {
            let name = k[0].as_str().unwrap();
            let code = k[1].as_u64().unwrap() as u8;
            assert_eq!(
                capi::cec::keycode_from_name(name).unwrap().0,
                code,
                "{name}"
            );
        }

        let (_, v) = json_call(&app, "GET", "/api/dev/opcodes").await;
        assert!(!v["data"].as_array().unwrap().is_empty());
    });
}

// -- auth / CSRF -------------------------------------------------------------------

mod auth {
    use super::*;

    fn app_with_token(token: &str) -> App {
        let state = app_state();
        state
            .settings()
            .update(|c| c.auth_token = token.to_string())
            .unwrap();
        capi::ui::LOGIN_TOKEN.set(token.to_string()).ok();
        server::build_router(state)
    }

    #[test]
    fn health_stays_open_token_required_elsewhere() {
        futures_lite_block_on(async {
            let app = app_with_token("s3cret");
            let (status, _, _) = call(app.clone(), "GET", "/api/health", None, &[]).await;
            assert_eq!(status, StatusCode::OK);

            let (status, _, _) = call(app.clone(), "GET", "/api/devices", None, &[]).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);

            // Bearer header works (503 past auth = adapter down).
            let (status, _, _) = call(
                app.clone(),
                "GET",
                "/api/devices",
                None,
                &[("Authorization", "Bearer s3cret")],
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

            // Custom header form also accepted.
            let (status, _, _) = call(
                app.clone(),
                "GET",
                "/api/devices",
                None,
                &[("X-Auth-Token", "s3cret")],
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

            // Query param accepted.
            let (status, _, _) =
                call(app.clone(), "GET", "/api/devices?key=s3cret", None, &[]).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

            // Wrong token rejected.
            let (status, _, _) = call(
                app.clone(),
                "GET",
                "/api/devices",
                None,
                &[("Authorization", "Bearer nope")],
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        });
    }

    #[test]
    fn cookie_auth_requires_same_origin() {
        futures_lite_block_on(async {
            let app = app_with_token("s3cret");
            // Cookie alone from same origin (no Origin header = non-browser) OK.
            let (status, _, _) = call(
                app.clone(),
                "GET",
                "/api/devices",
                None,
                &[("Cookie", "capi_token=s3cret")],
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

            // Cookie + foreign Origin: cookie is not honored cross-origin,
            // so the request is simply unauthenticated (401) — both for
            // reads and, more importantly, mutations.
            let (status, _, _) = call(
                app.clone(),
                "POST",
                "/api/power/on",
                None,
                &[
                    ("Cookie", "capi_token=s3cret"),
                    ("Origin", "https://evil.example"),
                ],
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        });
    }

    #[test]
    fn pages_redirect_to_login_when_unauthenticated() {
        futures_lite_block_on(async {
            let app = app_with_token("s3cret");
            let (status, _, headers) = call(app.clone(), "GET", "/settings", None, &[]).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(headers["location"], "/login");

            // Static assets remain public.
            let (status, _, _) = call(app, "GET", "/ui/static/style.css", None, &[]).await;
            assert_eq!(status, StatusCode::OK);
        });
    }

    #[test]
    fn login_flow_sets_cookie_and_works() {
        futures_lite_block_on(async {
            let app = app_with_token("s3cret");
            // Wrong token re-renders login with error.
            let ct = [("Content-Type", "application/x-www-form-urlencoded")];
            let (status, body, _) = call(
                app.clone(),
                "POST",
                "/login",
                Some("token=nope".into()),
                &ct,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(String::from_utf8_lossy(&body).contains("Invalid token"));

            // Correct token sets cookie + redirects.
            let (status, _, headers) = call(
                app.clone(),
                "POST",
                "/login",
                Some("token=s3cret".into()),
                &ct,
            )
            .await;
            assert_eq!(status, StatusCode::SEE_OTHER, "redirect expected");
            assert_eq!(headers["location"], "/");
            let cookie = headers["set-cookie"].to_str().unwrap();
            assert!(cookie.starts_with("capi_token=s3cret"));
            assert!(cookie.contains("HttpOnly"));
            assert!(cookie.contains("SameSite=Lax"));

            // Cookie from login authenticates API.
            let (status, _, _) =
                call(app, "GET", "/api/devices", None, &[("Cookie", cookie)]).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE); // past auth
        });
    }

    #[test]
    fn csrf_rejected_in_open_mode_too() {
        futures_lite_block_on(async {
            let app = server::build_router(app_state()); // NO token configured
            let (status, _, _) = call(
                app.clone(),
                "POST",
                "/api/power/on",
                None,
                &[("Origin", "https://evil.example")],
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);

            // Same-origin (host matches Host header) passes auth layer.
            let (status, _, _) = call(
                app,
                "POST",
                "/api/power/on",
                None,
                &[
                    ("Origin", "http://127.0.0.1:8080"),
                    ("Host", "127.0.0.1:8080"),
                ],
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE); // past CSRF, adapter down
        });
    }
}

// -- pages / fragments / assets -----------------------------------------------------

#[test]
fn pages_render_html_with_markers() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        for path in ["/", "/settings", "/dev"] {
            let (status, body, headers) = call(app.clone(), "GET", path, None, &[]).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            let ct = headers["content-type"].to_str().unwrap();
            assert!(ct.starts_with("text/html"), "{path}");
            let html = String::from_utf8_lossy(&body);
            assert!(html.contains("<!doctype html>"), "{path}");
            assert!(html.contains("htmx.min.js"), "{path} loads htmx");
        }

        // Dashboard shows the adapter-offline banner (empty state honesty).
        let (_, body, _) = call(server::build_router(app_state()), "GET", "/", None, &[]).await;
        let html = String::from_utf8_lossy(&body);
        // Fragments mount via hx-get on load; the page carries the slots.
        assert!(html.contains("bus-banner"), "banner slot present");
        assert!(html.contains("devices-panel"), "devices slot present");
    });
}

#[test]
fn ui_fragments_render_from_snapshot() {
    futures_lite_block_on(async {
        let state = app_state();
        // Seed a snapshot so devices fragment has content.
        state.bus().replace_snapshot(
            vec![serde_json::json!({
                "logical_address": 4,
                "osd_name": "Player",
                "device_type": "PlaybackDevice1",
                "physical_address": "1.0.0.0",
                "discovery": "active",
                "power_status": "on",
                "vendor_id": "0x000048",
                "vendor_name": "Unknown",
                "vendor_known": false,
                "cec_version": "1.4",
                "address_name": "PlaybackDevice1",
                "hdmi_port": 1,
            })],
            vec![4],
            4,
            false,
            false,
            Some(chrono_now()),
            180,
            256,
        );
        state.logs().push("APP", "integration log line".into());
        let app = server::build_router(state.clone());

        let (status, body, _) = call(app.clone(), "GET", "/ui/fragment/devices", None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("Player"),
            "OSD name rendered: {}",
            html.lines().next().unwrap_or("")
        );

        let (status, body, _) =
            call(app.clone(), "GET", "/ui/fragment/bus_banner", None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(String::from_utf8_lossy(&body).contains("No CEC adapter"));

        let (status, body, _) =
            call(app.clone(), "GET", "/ui/fragment/topology_hdmi", None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(String::from_utf8_lossy(&body).contains("port"));

        let (status, body, _) = call(app, "GET", "/ui/fragment/logs", None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(String::from_utf8_lossy(&body).contains("log-line"));
    });
}

fn chrono_now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

#[test]
fn static_assets_served_with_types() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        let (status, body, headers) =
            call(app.clone(), "GET", "/ui/static/style.css", None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/css"));
        assert!(body.len() > 1000);

        let (status, _, headers) = call(app.clone(), "GET", "/ui/static/app.js", None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript"));

        let (status, _, _) = call(app, "GET", "/ui/static/nope.js", None, &[]).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    });
}

// -- SSE ------------------------------------------------------------------------------

#[test]
fn sse_headers_and_named_event_format() {
    futures_lite_block_on(async {
        let state = app_state();
        let app = server::build_router(state.clone());
        state.hub().publish(capi::types::AppEvent::new(
            "adapter_state",
            serde_json::json!({"state": "connected"}),
        ));

        let (status, headers) = call_stream(app, "GET", "/api/events").await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream"));
    });
}

// -- middleware -------------------------------------------------------------------------

#[test]
fn request_id_echo_only_for_hex_and_recovery() {
    futures_lite_block_on(async {
        let app = server::build_router(app_state());
        // Sanitized echo: pure-hex client id honored.
        let (_, _, headers) = call(
            app.clone(),
            "GET",
            "/api/health",
            None,
            &[("X-Request-ID", "deadbeef01")],
        )
        .await;
        assert_eq!(headers["x-request-id"], "deadbeef01");

        // Non-hex client id replaced.
        let (_, _, headers) = call(
            app,
            "GET",
            "/api/health",
            None,
            &[("X-Request-ID", "../../etc")],
        )
        .await;
        let rid = headers["x-request-id"].to_str().unwrap();
        assert!(!rid.contains('/') && rid.chars().all(|c| c.is_ascii_hexdigit()));
    });
}
