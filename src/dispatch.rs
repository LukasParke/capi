//! Translation of raw CEC events and MQTT commands into app-side effects.
//! Shared by the binary entry point; `pub` so integration tests can drive
//! the exact production code path with synthetic events.

use crate::events::{EventHub, LogRing, Metrics};
use crate::settings;
use crate::strategies;
use crate::types::{self, AppEvent};
use std::sync::Arc;

use crate::busstate::BusState;
use crate::steward::Steward;
use std::sync::atomic::Ordering;

/// Translate a raw CEC event into app-side side effects (bus state, hub,
/// log ring, debounced steward hints).
pub fn dispatch_cec_event(
    conn: &Arc<crate::cec::Connection>,
    bus: &Arc<BusState>,
    hub: &Arc<EventHub>,
    steward: &Arc<Steward>,
    logs: &Arc<LogRing>,
    ev: crate::cec::CecEvent,
) {
    use crate::cec::{CecEvent as E, Opcode};
    match ev {
        E::Log { message, .. } => logs.push("CEC", message),
        E::KeyPress { key, duration } => {
            hub.publish(AppEvent::new(
                types::event_type::KEY_PRESS,
                serde_json::json!({"keycode": key, "duration": duration}),
            ));
        }
        E::Command(cmd) => {
            hub.publish(AppEvent::new(
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
            hub.publish(AppEvent::new(
                types::event_type::CONFIGURATION_CHANGED,
                serde_json::json!({"device_name": conn.device_name()}),
            ));
        }
        E::Alert {
            alert,
            param_type,
            param_value,
        } => {
            hub.publish(AppEvent::new(
                types::event_type::ALERT,
                serde_json::json!({"alert": alert, "param_type": param_type, "param": param_value}),
            ));
        }
        E::MenuState { .. } => {}
        E::SourceActivated { address, activated } => {
            hub.publish(AppEvent::new(
                types::event_type::SOURCE_ACTIVATED,
                serde_json::json!({"address": address, "activated": activated}),
            ));
            if activated {
                bus.update_active_source_quick(address as i32, true);
            }
        }
    }
}

/// Execute one inbound MQTT command through the shared exec helpers.
pub fn dispatch_mqtt_command(state: &crate::server::AppState, cmd: &crate::mqtt::MqttCommand) {
    if !state.adapter_ready() {
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
    let res: Result<String, crate::exec::ExecError> = match cmd.action.as_str() {
        "power/on" => crate::exec::power_on(state.adapter(), state.steward(), parse_addr(0))
            .map(|_| "ok".into()),
        "power/off" => crate::exec::power_off(state.adapter(), state.steward(), parse_addr(0))
            .map(|_| "ok".into()),
        "volume/up" => volume_via(state, strategies::Action::VolumeUp, None),
        "volume/down" => volume_via(state, strategies::Action::VolumeDown, None),
        "volume/mute" => volume_via(state, strategies::Action::Mute, None),
        "source" => {
            crate::exec::set_active_source(state.adapter(), state.steward(), parse_addr(-1))
                .map(|_| "ok".into())
        }
        "hdmi" => crate::exec::hdmi_port(state.adapter(), state.steward(), parse_addr(-1))
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
                Ok(k) => crate::exec::send_key(
                    state.adapter(),
                    state.bus(),
                    state.registry(),
                    k.address,
                    &k.key,
                    k.keycode,
                ),
                Err(e) => Err(crate::exec::ExecError::Other(format!(
                    "invalid key payload: {e}"
                ))),
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
    state: &crate::server::AppState,
    action: strategies::Action,
    addr: Option<i32>,
) -> Result<String, crate::exec::ExecError> {
    let conn = state
        .adapter()
        .get()
        .ok_or(crate::exec::ExecError::AdapterUnavailable)?;
    crate::exec::volume_action(&conn, state.bus(), state.registry(), action, addr)
}

/// Metrics hook used by main's hub counter task.
pub fn bump_events_published(m: &Arc<Metrics>) {
    m.events_published.fetch_add(1, Ordering::Relaxed);
}

/// Apply persisted per-vendor strategy overrides at boot.
pub fn apply_persisted_strategy_overrides(
    settings: &std::sync::Arc<settings::Settings>,
    registry: &strategies::Registry,
) {
    let overrides = settings.get().cec.strategy_overrides;
    for (vendor, by_action) in overrides {
        for (action_name, strat_name) in by_action {
            let Some(action) = strategies::Action::parse(&action_name) else {
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
