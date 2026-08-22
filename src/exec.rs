//! Shared CEC command helpers used by JSON API, UI actions, and MQTT.
//! Every path funnels through here so per-vendor strategy overrides apply
//! uniformly. Blocking calls; callers wrap in spawn_blocking.

use crate::adapter::AdapterHandle;
use crate::busstate::BusState;
use crate::cec::{self, CecError, Command, Connection, Keycode, LogicalAddress, Opcode};
use crate::strategies::{Action, Registry, RunOptions};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const ACTION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum ExecError {
    AdapterUnavailable,
    InvalidLogicalAddress,
    InvalidHdmiPort,
    InvalidKey,
    /// Neither key nor keycode supplied (keycode 0 = select by spec).
    MissingKey,
    Cec(crate::cec::CecError),
    Other(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::AdapterUnavailable => write!(f, "CEC adapter not available"),
            ExecError::InvalidLogicalAddress => write!(f, "invalid logical address"),
            ExecError::InvalidHdmiPort => write!(f, "invalid HDMI port"),
            ExecError::InvalidKey => write!(f, "invalid key"),
            ExecError::MissingKey => write!(
                f,
                "either 'key' or 'keycode' must be provided (keycode 0 = select; use key:\"select\")"
            ),
            ExecError::Cec(e) => write!(f, "{e:#}"),
            ExecError::Other(s) => write!(f, "{s}"),
        }
    }
}

pub fn vendor_id_for_target(bus: &BusState, target: LogicalAddress) -> String {
    if !target.is_valid() {
        return String::new();
    }
    let snap = bus.copy_snapshot();
    for d in &snap.devices {
        let la = d
            .get("logical_address")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if la == target.0 as i64 {
            return d
                .get("vendor_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
        }
    }
    String::new()
}

/// Run a registry action with a hard 5s cap. Returns a human summary.
pub fn run_action(
    conn: &Arc<Connection>,
    bus: &BusState,
    registry: &Registry,
    action: Action,
    target: Option<LogicalAddress>,
) -> Result<String, ExecError> {
    let vendor = target
        .map(|t| vendor_id_for_target(bus, t))
        .unwrap_or_default();
    let deadline = Instant::now() + ACTION_TIMEOUT;
    let results = registry.run(
        conn,
        bus,
        action,
        &RunOptions {
            vendor,
            target,
            ..Default::default()
        },
        deadline,
    );
    for r in &results {
        if r.status == crate::strategies::StratStatus::Ok {
            return Ok(format!(
                "{} ok via {} (reply {})",
                action.as_str(),
                r.strategy,
                r.reply_name
            ));
        }
    }
    match results.last() {
        Some(last) => Ok(format!(
            "{} tried {} strategies; last status={:?} strategy={}",
            action.as_str(),
            results.len(),
            last.status,
            last.strategy
        )),
        None => Err(ExecError::Other(format!(
            "no strategies for {}",
            action.as_str()
        ))),
    }
}

fn post_command_refresh(steward: &crate::steward::Steward) {
    steward.enqueue(crate::steward::JobKind::Light);
}

pub fn power_on(
    adapter: &AdapterHandle,
    steward: &crate::steward::Steward,
    addr: i32,
) -> Result<(), ExecError> {
    if !(0..=14).contains(&addr) {
        return Err(ExecError::InvalidLogicalAddress);
    }
    let c = adapter.get().ok_or(ExecError::AdapterUnavailable)?;
    c.power_on(LogicalAddress(addr as u8))
        .map_err(ExecError::Cec)?;
    post_command_refresh(steward);
    Ok(())
}

pub fn power_off(
    adapter: &AdapterHandle,
    steward: &crate::steward::Steward,
    addr: i32,
) -> Result<(), ExecError> {
    if !(0..=14).contains(&addr) {
        return Err(ExecError::InvalidLogicalAddress);
    }
    let c = adapter.get().ok_or(ExecError::AdapterUnavailable)?;
    c.standby(LogicalAddress(addr as u8))
        .map_err(ExecError::Cec)?;
    post_command_refresh(steward);
    Ok(())
}

pub fn power_status(adapter: &AdapterHandle, addr: i32) -> Result<String, ExecError> {
    if !(0..=14).contains(&addr) {
        return Err(ExecError::InvalidLogicalAddress);
    }
    let c = adapter.get().ok_or(ExecError::AdapterUnavailable)?;
    match c.get_device_power_status(LogicalAddress(addr as u8)) {
        Ok(p) => Ok(p.to_string()),
        Err(e) => Err(ExecError::Cec(e)),
    }
}

/// Volume/mute through the registry. `addr` None = default chain
/// (AudioSystem → TV → Playback1 → libcec), Some(la) forces the target.
pub fn volume_action(
    conn: &Arc<Connection>,
    bus: &BusState,
    registry: &Registry,
    action: Action,
    addr: Option<i32>,
) -> Result<String, ExecError> {
    match addr {
        None => run_action(conn, bus, registry, action, None),
        Some(a) => {
            if !(0..=14).contains(&a) {
                return Err(ExecError::InvalidLogicalAddress);
            }
            run_action(conn, bus, registry, action, Some(LogicalAddress(a as u8)))
        }
    }
}

pub fn set_active_source(
    adapter: &AdapterHandle,
    steward: &crate::steward::Steward,
    addr: i32,
) -> Result<(), ExecError> {
    if !(0..=14).contains(&addr) {
        return Err(ExecError::InvalidLogicalAddress);
    }
    let c = adapter.get().ok_or(ExecError::AdapterUnavailable)?;
    switch_to_device(&c, LogicalAddress(addr as u8)).map_err(ExecError::Cec)?;
    post_command_refresh(steward);
    Ok(())
}

pub fn hdmi_port(
    adapter: &AdapterHandle,
    steward: &crate::steward::Steward,
    port: i32,
) -> Result<(), ExecError> {
    if !(1..=15).contains(&port) {
        return Err(ExecError::InvalidHdmiPort);
    }
    let c = adapter.get().ok_or(ExecError::AdapterUnavailable)?;
    switch_to_hdmi_port(&c, port as u8).map_err(ExecError::Cec)?;
    post_command_refresh(steward);
    Ok(())
}

/// Port switch: libcec SetHDMIPort first, ActiveSource broadcast fallback
/// (parity with Go SwitchToHDMIPort).
pub fn switch_to_hdmi_port(c: &Connection, port: u8) -> Result<(), CecError> {
    if !(1..=15).contains(&port) {
        return Err(CecError::InvalidHdmiPort);
    }
    if c.set_hdmi_port(LogicalAddress::TV, port).is_ok() {
        return Ok(());
    }
    let phys = (port as u16) << 12;
    c.transmit(&Command {
        initiator: c
            .first_logical_address()
            .unwrap_or(LogicalAddress::FREE_USE),
        destination: LogicalAddress::BROADCAST,
        opcode: Opcode::ACTIVE_SOURCE,
        opcode_set: true,
        parameters: vec![(phys >> 8) as u8, (phys & 0xFF) as u8],
        ack: false,
        eom: true,
    })
}

/// Device switch: ActiveSource broadcast of the device's physical address.
pub fn switch_to_device(c: &Connection, address: LogicalAddress) -> Result<(), CecError> {
    if !address.is_valid() {
        return Err(CecError::InvalidLogicalAddress);
    }
    let phys = c.get_device_physical_address(address).map_err(|e| {
        tracing::debug!("switch to {}: physical lookup failed: {e:#}", address.0);
        e
    })?;
    c.transmit(&Command {
        initiator: c
            .first_logical_address()
            .unwrap_or(LogicalAddress::FREE_USE),
        destination: LogicalAddress::BROADCAST,
        opcode: Opcode::ACTIVE_SOURCE,
        opcode_set: true,
        parameters: vec![(phys >> 8) as u8, (phys & 0xFF) as u8],
        ack: false,
        eom: true,
    })
}

/// Validate key arguments without touching the adapter, so HTTP handlers
/// can return precise 400s even when the bus is down.
pub fn validate_key_args(addr: i32, key_name: &str, keycode: i32) -> Result<(), ExecError> {
    if !(0..=14).contains(&addr) {
        return Err(ExecError::InvalidLogicalAddress);
    }
    if !key_name.is_empty() {
        if key_to_action(key_name).is_some() || cec::keycode_from_name(key_name).is_some() {
            return Ok(());
        }
        return Err(ExecError::InvalidKey);
    }
    if keycode != 0 {
        if !(0..=255).contains(&keycode) {
            return Err(ExecError::InvalidKey);
        }
        return Ok(());
    }
    Err(ExecError::MissingKey)
}

/// Send a key by canonical name or raw keycode. keycode 0 is rejected —
/// it means Select and must be sent as key:"select" (documented parity).
pub fn send_key(
    adapter: &AdapterHandle,
    bus: &BusState,
    registry: &Registry,
    addr: i32,
    key_name: &str,
    keycode: i32,
) -> Result<String, ExecError> {
    validate_key_args(addr, key_name, keycode)?;
    let conn = adapter.get().ok_or(ExecError::AdapterUnavailable)?;

    // Route keys that map to registry actions through it (vendor overrides).
    if !key_name.is_empty() {
        if let Some(action) = key_to_action(key_name) {
            return run_action(
                &conn,
                bus,
                registry,
                action,
                Some(LogicalAddress(addr as u8)),
            );
        }
        let key = cec::keycode_from_name(key_name).ok_or(ExecError::InvalidKey)?;
        conn.send_keypress(LogicalAddress(addr as u8), key, true)
            .map_err(ExecError::Cec)?;
        return Ok("key sent".into());
    }
    if keycode != 0 {
        if !(0..=255).contains(&keycode) {
            return Err(ExecError::InvalidKey);
        }
        conn.send_keypress(LogicalAddress(addr as u8), Keycode(keycode as u8), true)
            .map_err(ExecError::Cec)?;
        return Ok("key sent".into());
    }
    Err(ExecError::Other(
        "either 'key' or 'keycode' must be provided (keycode 0 = select; use key:\"select\")"
            .into(),
    ))
}

/// Keys with dedicated registry actions route through strategies.
fn key_to_action(name: &str) -> Option<Action> {
    let n = name.trim().to_lowercase().replace(['-', ' '], "_");
    Some(match n.as_str() {
        "volume_up" => Action::VolumeUp,
        "volume_down" => Action::VolumeDown,
        "mute" => Action::Mute,
        "up" | "nav_up" => Action::NavUp,
        "down" | "nav_down" => Action::NavDown,
        "left" | "nav_left" => Action::NavLeft,
        "right" | "nav_right" => Action::NavRight,
        "select" | "enter" => Action::Select,
        "back" | "exit" => Action::Back,
        "home" | "root_menu" => Action::Home,
        "menu" | "setup_menu" => Action::Menu,
        "channel_up" => Action::ChannelUp,
        "channel_down" => Action::ChannelDown,
        "play" => Action::Play,
        "pause" => Action::Pause,
        "stop" => Action::Stop,
        "fast_forward" => Action::FastForward,
        "rewind" => Action::Rewind,
        "record" => Action::Record,
        _ => return None,
    })
}

pub fn audio_status(adapter: &AdapterHandle) -> (u8, bool, u8) {
    match adapter.get().map(|c| c.get_audio_status()) {
        Some(Ok(t)) => t,
        _ => (0, false, 0),
    }
}
