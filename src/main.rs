//! capi — HDMI-CEC control over HTTP and MQTT (Rust).

mod adapter;
mod assets;
mod busstate;
#[allow(dead_code)]
mod cec;
mod events;
mod exec;
mod mqtt;
mod server;
mod settings;
mod steward;
mod strategies;
mod supervisor;
mod topology;
mod types;
mod ui;
mod ui_ctx;
mod update;
mod util;

use adapter::AdapterHandle;
use busstate::BusState;
use events::{EventHub, LogRing, Metrics};
use settings::Settings;
use std::sync::Arc;
use steward::Steward;
use strategies::Registry;

fn main() {
    settings::init_tracing();

    let flags = match settings::parse_flags(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "capi: {e}\nusage: capi [-bind :8080] [-name N] [-adapter PATH] [-token T]\n             [-mqtt-broker URL] [-mqtt-user U] [-mqtt-pass P] [-mqtt-prefix P]\n             [-cec-monitor] [-version] [-update]"
            );
            std::process::exit(2);
        }
    };
    if flags.show_version {
        println!("{}", ui::VERSION);
        return;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async_main(flags));
}

async fn async_main(flags: settings::Flags) {
    // ---- config ------------------------------------------------------------
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("./capi"));
    let config_path = exe.parent().unwrap().join("config.json");
    let (settings, _corrupt_note) = match Settings::load(&config_path) {
        Ok((s, c)) => (Arc::new(s), c),
        Err(e) => {
            // Quarantine + refuse: never silently run with defaults over a
            // file the user wrote (fixes the Go silent-reset data loss).
            tracing::error!("{e}");
            Settings::quarantine_corrupt(&config_path);
            eprintln!(
                "capi: refusing to start; config quarantined as {}.corrupt",
                config_path.display()
            );
            std::process::exit(1);
        }
    };

    settings.apply_overrides(&settings::CliOverrides {
        mqtt_broker: (!flags.mqtt_broker.is_empty()).then(|| flags.mqtt_broker.clone()),
        mqtt_user: (!flags.mqtt_user.is_empty()).then(|| flags.mqtt_user.clone()),
        mqtt_pass: (!flags.mqtt_pass.is_empty()).then(|| flags.mqtt_pass.clone()),
        mqtt_prefix_explicit: std::env::args()
            .any(|a| a == "-mqtt-prefix" || a.starts_with("-mqtt-prefix=")),
        mqtt_prefix: flags.mqtt_prefix.clone(),
        token: (!flags.token.is_empty()).then(|| flags.token.clone()),
    });
    let bind = flags.bind.clone();

    if settings.get().auth_token.is_empty() {
        tracing::warn!("no auth token configured — API is open on the LAN. Set auth_token in config.json or pass -token.");
    }
    ui::LOGIN_TOKEN.set(settings.get().auth_token.clone()).ok();

    // ---- shared state ------------------------------------------------------
    let hub = Arc::new(EventHub::new(512));
    let logs = LogRing::new(500);
    let bus = Arc::new(BusState::new());
    let metrics = Arc::new(Metrics::default());
    let adapter = AdapterHandle::new();
    let registry = Arc::new(Registry::new());

    apply_persisted_strategy_overrides(&settings, &registry);

    let steward = Arc::new(Steward::spawn(
        bus.clone(),
        hub.clone(),
        settings.clone(),
        adapter.clone(),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    ));

    // ---- MQTT --------------------------------------------------------------
    let mqtt = mqtt::MqttHandle::new();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<mqtt::MqttCommand>();
    let _ = server::MQTT_CMD_TX.set(cmd_tx.clone());
    mqtt.start(settings.get().mqtt, hub.subscribe(), cmd_tx);

    let state = server::AppState::new(
        settings.clone(),
        hub.clone(),
        logs.clone(),
        bus.clone(),
        adapter.clone(),
        steward.clone(),
        registry.clone(),
        metrics.clone(),
        mqtt.clone(),
    );
    {
        let st = state.clone();
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                let st = st.clone();
                tokio::task::spawn_blocking(move || dispatch_mqtt_command(&st, &cmd));
            }
        });
    }

    // ---- supervisor thread --------------------------------------------------
    {
        let deps = supervisor::SupervisorDeps {
            settings: settings.clone(),
            adapter: adapter.clone(),
            bus: bus.clone(),
            hub: hub.clone(),
        };
        let logs2 = logs.clone();
        let steward2 = steward.clone();
        let name = flags.name.clone();
        let adapter_path = flags.adapter.clone();
        let monitor = flags.cec_monitor;
        std::thread::Builder::new()
            .name("supervisor".into())
            .spawn(move || {
                supervisor::run_supervisor(
                    deps,
                    name,
                    adapter_path,
                    monitor,
                    std::sync::Arc::new(
                        move |conn: &std::sync::Arc<crate::cec::Connection>,
                              ev: crate::cec::CecEvent| {
                            dispatch_cec_event(conn, &bus, &hub, &steward2, &logs2, ev)
                        },
                    ),
                )
            })
            .expect("spawn supervisor");
    }

    // ---- HTTP server --------------------------------------------------------
    let app = server::build_router(state.clone());
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|e| {
            eprintln!("capi: cannot bind {bind}: {e}");
            std::process::exit(1);
        });
    tracing::info!("capi {} listening on http://{bind}", ui::VERSION);

    // Keepalive loop: re-apply MQTT config if broker configured but down.
    {
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let cfg = st.0.settings.get().mqtt;
                if !cfg.broker.is_empty() && !st.mqtt_connected() {
                    st.apply_mqtt_config(&cfg);
                }
            }
        });
    }

    let adapter_for_shutdown = adapter.clone();
    let shutdown = async move {
        let ctrl_c = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        #[cfg(unix)]
        let term = async {
            use tokio::signal::unix::{signal, SignalKind};
            match signal(SignalKind::terminate()) {
                Ok(mut s) => {
                    s.recv().await;
                }
                Err(_) => std::future::pending::<()>().await,
            }
        };
        #[cfg(not(unix))]
        let term = std::future::pending::<()>();
        tokio::select! {
            _ = ctrl_c => {},
            _ = term => {},
        }
        tracing::info!("shutting down");
        supervisor::SHUTDOWN_FLAG.store(true, std::sync::atomic::Ordering::SeqCst);
        adapter_for_shutdown.signal_reconnect();
    };

    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
    {
        tracing::error!("server: {e}");
    }
    mqtt.stop();
}

fn apply_persisted_strategy_overrides(settings: &Arc<Settings>, registry: &Registry) {
    let overrides = settings.get().cec.strategy_overrides;
    for (vendor, by_action) in overrides {
        for (action_name, strat_name) in by_action {
            let Some(action) = crate::strategies::Action::parse(&action_name) else {
                tracing::warn!("config: unknown action {action_name:?} in strategy override");
                continue;
            };
            let picked = registry
                .strategies_for("", action)
                .into_iter()
                .find(|s| s.name == strat_name);
            match picked {
                Some(s) => {
                    registry.set_vendor_override(&vendor, action, vec![s]);
                    tracing::info!("applied strategy override vendor={vendor} action={action_name} strategy={strat_name}");
                }
                None => {
                    tracing::warn!("config: strategy {strat_name:?} not found for {action_name}")
                }
            }
        }
    }
}

fn dispatch_mqtt_command(state: &server::AppState, cmd: &mqtt::MqttCommand) {
    if !state.0.adapter.ready() {
        tracing::warn!(
            "[MQTT] ignoring command {:?}: CEC adapter not available",
            cmd.action
        );
        return;
    }
    let parse_addr = |dflt: i32| -> i32 {
        std::str::from_utf8(&cmd.payload)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(dflt)
    };
    let res: Result<String, exec::ExecError> =
        match cmd.action.as_str() {
            "power/on" => exec::power_on(&state.0.adapter, &state.0.steward, parse_addr(0))
                .map(|_| "ok".into()),
            "power/off" => exec::power_off(&state.0.adapter, &state.0.steward, parse_addr(0))
                .map(|_| "ok".into()),
            "volume/up" => volume_via(state, strategies::Action::VolumeUp, None),
            "volume/down" => volume_via(state, strategies::Action::VolumeDown, None),
            "volume/mute" => volume_via(state, strategies::Action::Mute, None),
            "source" => exec::set_active_source(&state.0.adapter, &state.0.steward, parse_addr(-1))
                .map(|_| "ok".into()),
            "hdmi" => exec::hdmi_port(&state.0.adapter, &state.0.steward, parse_addr(-1))
                .map(|_| "ok".into()),
            "key" => {
                #[derive(serde::Deserialize)]
                struct K {
                    #[serde(default)]
                    address: i32,
                    #[serde(default)]
                    key: String,
                    #[serde(default)]
                    keycode: i32,
                }
                match serde_json::from_slice::<K>(&cmd.payload) {
                    Ok(k) => exec::send_key(
                        &state.0.adapter,
                        &state.0.bus,
                        &state.0.registry,
                        k.address,
                        &k.key,
                        k.keycode,
                    ),
                    Err(e) => Err(exec::ExecError::Other(format!("invalid key payload: {e}"))),
                }
            }
            other => {
                tracing::warn!("[MQTT] unknown command topic: {other}");
                return;
            }
        };
    match res {
        Ok(msg) => tracing::info!("[MQTT] {}: {msg}", cmd.action),
        Err(e) => tracing::warn!("[MQTT] {} failed: {e}", cmd.action),
    }
}

fn volume_via(
    state: &server::AppState,
    action: strategies::Action,
    addr: Option<i32>,
) -> Result<String, exec::ExecError> {
    let conn = state
        .0
        .adapter
        .get()
        .ok_or(exec::ExecError::AdapterUnavailable)?;
    exec::volume_action(&conn, &state.0.bus, &state.0.registry, action, addr)
}

/// Translate a raw CEC event into app-side side effects (bus state, hub,
/// log ring, debounced steward hints).
fn dispatch_cec_event(
    conn: &Arc<cec::Connection>,
    bus: &Arc<BusState>,
    hub: &Arc<EventHub>,
    steward: &Arc<Steward>,
    logs: &Arc<LogRing>,
    ev: cec::CecEvent,
) {
    use cec::{CecEvent as E, Opcode};
    match ev {
        E::Log { message, .. } => logs.push("CEC", message),
        E::KeyPress { key, duration } => {
            hub.publish(types::AppEvent::new(
                types::event_type::KEY_PRESS,
                serde_json::json!({"keycode": key, "duration": duration}),
            ));
        }
        E::Command(cmd) => {
            hub.publish(types::AppEvent::new(
                types::event_type::COMMAND,
                serde_json::json!({
                    "initiator": cmd.initiator.0,
                    "destination": cmd.destination.0,
                    "opcode": format!("0x{:02X}", cmd.opcode.0),
                }),
            ));
            if cmd.initiator.0 <= 14 {
                bus.note_seen(cmd.initiator.0 as i32);
            }
            bus.apply_observed_command(&cmd);
            let cap = bus.frame_ring_capacity();
            if cap > 0 {
                bus.append_frame(&cmd, cap);
            }
            let heavy = matches!(
                cmd.opcode,
                Opcode::REPORT_PHYSICAL_ADDRESS
                    | Opcode::DEVICE_VENDOR_ID
                    | Opcode::SET_OSD_NAME
                    | Opcode::ACTIVE_SOURCE
                    | Opcode::ROUTING_CHANGE
                    | Opcode::ROUTING_INFORMATION
                    | Opcode::SET_STREAM_PATH
                    | Opcode::INACTIVE_SOURCE
                    | Opcode::REQUEST_ACTIVE_SOURCE
            );
            let light = matches!(
                cmd.opcode,
                Opcode::REPORT_POWER_STATUS | Opcode::REPORT_AUDIO_STATUS
            );
            if heavy || light {
                steward.hint(heavy);
            }
        }
        E::ConfigurationChanged(_) => {
            hub.publish(types::AppEvent::new(
                types::event_type::CONFIGURATION_CHANGED,
                serde_json::json!({"device_name": conn.device_name()}),
            ));
        }
        E::Alert {
            alert,
            param_type,
            param_value,
        } => {
            hub.publish(types::AppEvent::new(
                types::event_type::ALERT,
                serde_json::json!({"alert": alert, "param_type": param_type, "param": param_value}),
            ));
        }
        E::MenuState { .. } => {}
        E::SourceActivated { address, activated } => {
            hub.publish(types::AppEvent::new(
                types::event_type::SOURCE_ACTIVATED,
                serde_json::json!({"address": address, "activated": activated}),
            ));
            if activated {
                bus.update_active_source_quick(address as i32, true);
            }
        }
    }
}
