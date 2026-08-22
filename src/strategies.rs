//! Action → strategy-chain registry with per-vendor overrides.
//!
//! Fixes vs Go: single-flight around `run` per connection (concurrent runs no
//! longer interleave frame-ring diffs), all client-tunable sleeps clamped,
//! observe windows derive from caller contexts, ring diff by sequence number.

use crate::busstate::BusState;
use crate::cec::{self, Command, Connection, Keycode, LogicalAddress, Opcode};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    VolumeUp,
    VolumeDown,
    Mute,
    NavUp,
    NavDown,
    NavLeft,
    NavRight,
    Select,
    Back,
    Home,
    Menu,
    ChannelUp,
    ChannelDown,
    Play,
    Pause,
    Stop,
    FastForward,
    Rewind,
    Record,
    Power,
    Number(u8),
}

pub const ALL_ACTIONS: &[(&str, Action)] = &[
    ("volume_up", Action::VolumeUp),
    ("volume_down", Action::VolumeDown),
    ("mute", Action::Mute),
    ("nav_up", Action::NavUp),
    ("nav_down", Action::NavDown),
    ("nav_left", Action::NavLeft),
    ("nav_right", Action::NavRight),
    ("select", Action::Select),
    ("back", Action::Back),
    ("home", Action::Home),
    ("menu", Action::Menu),
    ("channel_up", Action::ChannelUp),
    ("channel_down", Action::ChannelDown),
    ("play", Action::Play),
    ("pause", Action::Pause),
    ("stop", Action::Stop),
    ("fast_forward", Action::FastForward),
    ("rewind", Action::Rewind),
    ("record", Action::Record),
    ("power", Action::Power),
    ("number_0", Action::Number(0)),
    ("number_1", Action::Number(1)),
    ("number_2", Action::Number(2)),
    ("number_3", Action::Number(3)),
    ("number_4", Action::Number(4)),
    ("number_5", Action::Number(5)),
    ("number_6", Action::Number(6)),
    ("number_7", Action::Number(7)),
    ("number_8", Action::Number(8)),
    ("number_9", Action::Number(9)),
];

impl Action {
    pub fn as_str(self) -> &'static str {
        ALL_ACTIONS
            .iter()
            .find(|(_, a)| *a == self)
            .map(|(s, _)| *s)
            .unwrap_or("unknown")
    }

    pub fn parse(s: &str) -> Option<Action> {
        let norm = s.trim().to_lowercase().replace(['-', ' '], "_");
        ALL_ACTIONS
            .iter()
            .find(|(n, _)| *n == norm)
            .map(|(_, a)| *a)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum StepKind {
    SendUserControl,
    Transmit,
    #[allow(dead_code)]
    LibcecVolumeUp,
    LibcecVolumeDown,
    LibcecMute,
    LibcecPowerOn,
    LibcecStandby,
    EnableSam,
    Wait,
}

impl StepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StepKind::SendUserControl => "send_user_control",
            StepKind::Transmit => "transmit",
            StepKind::LibcecVolumeUp => "libcec_volume_up",
            StepKind::LibcecVolumeDown => "libcec_volume_down",
            StepKind::LibcecMute => "libcec_mute",
            StepKind::LibcecPowerOn => "libcec_power_on",
            StepKind::LibcecStandby => "libcec_standby",
            StepKind::EnableSam => "enable_sam",
            StepKind::Wait => "wait",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Step {
    pub kind: StepKind,
    pub target: LogicalAddress,
    pub key: Keycode,
    pub wait: bool,
    pub hold_ms: i64,
    pub opcode: Opcode,
    pub params: Vec<u8>,
    pub delay_ms: i64,
}

impl Step {
    fn uc(target: LogicalAddress, key: Keycode, hold_ms: i64) -> Step {
        Step {
            kind: StepKind::SendUserControl,
            target,
            key,
            wait: true,
            hold_ms,
            opcode: Opcode(0),
            params: Vec::new(),
            delay_ms: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Strategy {
    pub name: String,
    pub steps: Vec<Step>,
    pub observe_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StratStatus {
    Ok,
    AckedNoReply,
    FeatureAborted,
    NoAck,
    Error,
    Skipped,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StepResult {
    pub kind: String,
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub target: i32,
    pub acked: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StratResult {
    pub strategy: String,
    pub status: StratStatus,
    pub acked: bool,
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub reply_opcode: i32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reply_name: String,
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub abort_opcode: i32,
    pub elapsed_ms: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StepResult>,
}

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub vendor: String,
    pub target: Option<LogicalAddress>,
    pub all_strategies: bool,
    pub observe_override_ms: i64,
}

// Hard caps on client-tunable timing (fix: unbounded request-driven sleeps).
pub const MAX_OBSERVE_MS: i64 = 5000;
pub const MAX_HOLD_MS: i64 = 2000;
pub const MAX_DELAY_MS: i64 = 2000;

fn clamp_ms(v: i64, max: i64) -> i64 {
    v.clamp(0, max)
}

pub struct Registry {
    inner: RwLock<RegistryInner>,
    /// Single-flight around run() so two concurrent actions cannot interleave
    /// frame-ring observations (Go assumed this never happened; enforce it).
    run_lock: Mutex<()>,
}

struct RegistryInner {
    defaults: HashMap<Action, Vec<Strategy>>,
    per_vendor: HashMap<String, HashMap<Action, Vec<Strategy>>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(RegistryInner {
                defaults: default_strategies(),
                per_vendor: HashMap::new(),
            }),
            run_lock: Mutex::new(()),
        }
    }

    pub fn set_vendor_override(&self, vendor: &str, action: Action, strategies: Vec<Strategy>) {
        let v = vendor.trim().to_lowercase();
        if v.is_empty() {
            return;
        }
        let mut g = self.inner.write().expect("registry lock");
        let entry = g.per_vendor.entry(v).or_default();
        if strategies.is_empty() {
            entry.remove(&action);
        } else {
            entry.insert(action, strategies);
        }
    }

    pub fn strategies_for(&self, vendor: &str, action: Action) -> Vec<Strategy> {
        let g = self.inner.read().expect("registry lock");
        if !vendor.is_empty() {
            if let Some(per) = g.per_vendor.get(vendor.trim().to_lowercase().as_str()) {
                if let Some(s) = per.get(&action) {
                    return s.clone();
                }
            }
        }
        g.defaults.get(&action).cloned().unwrap_or_default()
    }

    pub fn default_chain(&self, action: Action) -> Vec<(String, Vec<StepKind>)> {
        self.strategies_for("", action)
            .into_iter()
            .map(|s| (s.name, s.steps.iter().map(|st| st.kind).collect()))
            .collect()
    }

    /// Execute the chain for `action`. Blocking; call from spawn_blocking.
    pub fn run(
        &self,
        conn: &Arc<Connection>,
        bus: &BusState,
        action: Action,
        opts: &RunOptions,
        deadline: Instant,
    ) -> Vec<StratResult> {
        // Parity with Go: monitor-only sessions refuse strategy runs.
        if conn.is_monitor_only() {
            return vec![StratResult {
                strategy: "monitor-only".into(),
                status: StratStatus::Skipped,
                acked: false,
                reply_opcode: 0,
                reply_name: String::new(),
                abort_opcode: 0,
                elapsed_ms: 0,
                error: "connection is monitor-only".into(),
                steps: vec![],
            }];
        }
        let _guard = self.run_lock.lock().expect("run lock");
        let chain = self.strategies_for(&opts.vendor, action);
        if chain.is_empty() {
            return vec![StratResult {
                strategy: action.as_str().to_string(),
                status: StratStatus::Skipped,
                acked: false,
                reply_opcode: 0,
                reply_name: String::new(),
                abort_opcode: 0,
                elapsed_ms: 0,
                error: format!("no strategies registered for {}", action.as_str()),
                steps: Vec::new(),
            }];
        }

        let own_la = conn
            .first_logical_address()
            .map(|a| a.0 as i32)
            .unwrap_or(-1);
        let mut results = Vec::with_capacity(chain.len());
        for s in chain {
            if Instant::now() >= deadline {
                break;
            }
            let res = execute_strategy(conn, bus, &s, opts, deadline, own_la);
            results.push(res.clone());
            if !opts.all_strategies && res.status == StratStatus::Ok {
                break;
            }
        }
        results
    }
}

fn execute_strategy(
    conn: &Arc<Connection>,
    bus: &BusState,
    s: &Strategy,
    opts: &RunOptions,
    deadline: Instant,
    own_la: i32,
) -> StratResult {
    let start = std::time::Instant::now();
    let mut res = StratResult {
        strategy: s.name.clone(),
        status: StratStatus::AckedNoReply,
        acked: true,
        reply_opcode: 0,
        reply_name: String::new(),
        abort_opcode: 0,
        elapsed_ms: 0,
        error: String::new(),
        steps: Vec::new(),
    };

    let pre_seq = bus.ring_high_water();
    let mut last_expected: Option<Opcode> = None;

    for st in &s.steps {
        let mut target = st.target;
        let mut step_res = StepResult {
            kind: st.kind.as_str().to_string(),
            target: target.0 as i32,
            acked: false,
            error: String::new(),
        };
        if target == LogicalAddress::UNKNOWN {
            if let Some(t) = opts.target {
                target = t;
                step_res.target = target.0 as i32;
            }
        }

        if st.kind == StepKind::Wait {
            let d = Duration::from_millis(clamp_ms(st.delay_ms, MAX_DELAY_MS) as u64);
            if !sleep_until(deadline, d) {
                res.status = StratStatus::Error;
                res.error = "deadline exceeded".into();
                res.elapsed_ms = start.elapsed().as_millis() as i64;
                return res;
            }
            res.steps.push(step_res);
            continue;
        }

        match execute_step(conn, st, target) {
            Ok(()) => {
                step_res.acked = true;
                if let Some(exp) = expected_reply_opcode(st) {
                    last_expected = Some(exp);
                }
            }
            Err(e) => {
                step_res.error = format!("{e:#}");
                res.acked = false;
            }
        }
        res.steps.push(step_res);

        if st.delay_ms > 0 {
            let d = Duration::from_millis(clamp_ms(st.delay_ms, MAX_DELAY_MS) as u64);
            if !sleep_until(deadline, d) {
                res.status = StratStatus::Error;
                res.error = "deadline exceeded".into();
                res.elapsed_ms = start.elapsed().as_millis() as i64;
                return res;
            }
        }
    }

    let observe = if opts.observe_override_ms > 0 {
        clamp_ms(opts.observe_override_ms, MAX_OBSERVE_MS)
    } else if s.observe_ms > 0 {
        clamp_ms(s.observe_ms, MAX_OBSERVE_MS)
    } else {
        500
    };
    if !sleep_until(deadline, Duration::from_millis(observe as u64)) {
        res.status = StratStatus::Error;
        res.error = "deadline exceeded".into();
        res.elapsed_ms = start.elapsed().as_millis() as i64;
        return res;
    }

    let new_frames = bus.frames_after(pre_seq);
    classify(&mut res, &new_frames, last_expected, own_la);
    res.elapsed_ms = start.elapsed().as_millis() as i64;
    res
}

/// Sleep `d`, returning false when `deadline` would be exceeded first.
fn sleep_until(deadline: Instant, d: Duration) -> bool {
    let end = std::time::Instant::now() + d;
    if end > deadline {
        return false;
    }
    std::thread::sleep(d);
    true
}

fn execute_step(
    conn: &Arc<Connection>,
    st: &Step,
    target: LogicalAddress,
) -> Result<(), cec::CecError> {
    match st.kind {
        StepKind::SendUserControl => {
            conn.send_keypress(target, st.key, st.wait)?;
            if st.hold_ms > 0 {
                std::thread::sleep(Duration::from_millis(
                    clamp_ms(st.hold_ms, MAX_HOLD_MS) as u64
                ));
                conn.send_key_release(target, st.wait)?;
            }
            Ok(())
        }
        StepKind::Transmit => conn.transmit(&Command {
            initiator: conn
                .first_logical_address()
                .unwrap_or(LogicalAddress::FREE_USE),
            destination: target,
            opcode: st.opcode,
            opcode_set: true,
            parameters: st.params.clone(),
            ack: false,
            eom: true,
        }),
        StepKind::LibcecVolumeUp => conn.volume_up(true),
        StepKind::LibcecVolumeDown => conn.volume_down(true),
        StepKind::LibcecMute => conn.audio_toggle_mute(),
        StepKind::LibcecPowerOn => conn.power_on(target),
        StepKind::LibcecStandby => conn.standby(target),
        StepKind::EnableSam => conn.set_system_audio_mode(true),
        StepKind::Wait => Ok(()),
    }
}

fn expected_reply_opcode(s: &Step) -> Option<Opcode> {
    match s.kind {
        StepKind::SendUserControl => match s.key {
            Keycode::VOLUME_UP | Keycode::VOLUME_DOWN | Keycode::MUTE => {
                Some(Opcode::REPORT_AUDIO_STATUS)
            }
            _ => None,
        },
        StepKind::LibcecVolumeUp | StepKind::LibcecVolumeDown | StepKind::LibcecMute => {
            Some(Opcode::REPORT_AUDIO_STATUS)
        }
        StepKind::LibcecPowerOn | StepKind::LibcecStandby => Some(Opcode::REPORT_POWER_STATUS),
        StepKind::Transmit => match s.opcode {
            Opcode::GIVE_DEVICE_POWER_STATUS => Some(Opcode::REPORT_POWER_STATUS),
            Opcode::GIVE_AUDIO_STATUS => Some(Opcode::REPORT_AUDIO_STATUS),
            Opcode::GIVE_DEVICE_VENDOR_ID => Some(Opcode::DEVICE_VENDOR_ID),
            Opcode::GIVE_OSD_NAME => Some(Opcode::SET_OSD_NAME),
            Opcode::GIVE_PHYSICAL_ADDRESS => Some(Opcode::REPORT_PHYSICAL_ADDRESS),
            Opcode::GET_CEC_VERSION => Some(Opcode::CEC_VERSION),
            _ => None,
        },
        _ => None,
    }
}

fn classify(
    res: &mut StratResult,
    new_frames: &[crate::types::BusFrameEntry],
    expected: Option<Opcode>,
    own_la: i32,
) {
    for f in new_frames {
        if f.initiator == own_la {
            continue;
        }
        let op = parse_hex_byte(&f.opcode);
        if op as u8 == Opcode::FEATURE_ABORT.0 && !f.params_hex.is_empty() {
            res.status = StratStatus::FeatureAborted;
            res.abort_opcode =
                parse_hex_byte(f.params_hex.first().map(String::as_str).unwrap_or("0"));
            res.reply_opcode = op;
            res.reply_name = "FEATURE_ABORT".into();
            return;
        }
        if let Some(exp) = expected {
            if op as u8 == exp.0 {
                res.status = StratStatus::Ok;
                res.reply_opcode = op;
                res.reply_name = cec::opcode_name(exp);
                return;
            }
        }
    }
    if !res.acked {
        res.status = StratStatus::NoAck;
        return;
    }
    res.status = StratStatus::AckedNoReply;
}

fn parse_hex_byte(s: &str) -> i32 {
    let t = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    i32::from_str_radix(t, 16).unwrap_or(0) & 0xFF
}

fn uc_press(name: impl Into<String>, target: LogicalAddress, key: Keycode, hold: i64) -> Strategy {
    Strategy {
        name: name.into(),
        steps: vec![Step::uc(target, key, hold)],
        observe_ms: 0,
    }
}

fn libcec_volume(name: &str, kind: StepKind) -> Strategy {
    Strategy {
        name: name.into(),
        steps: vec![Step {
            kind,
            target: LogicalAddress::UNKNOWN,
            key: Keycode(0),
            wait: false,
            hold_ms: 0,
            opcode: Opcode(0),
            params: Vec::new(),
            delay_ms: 0,
        }],
        observe_ms: 0,
    }
}

const VOL_HOLD: i64 = 250;
const NAV_HOLD: i64 = 100;

fn default_strategies() -> HashMap<Action, Vec<Strategy>> {
    use Action::*;
    use LogicalAddress as LA;
    let mut m: HashMap<Action, Vec<Strategy>> = HashMap::new();
    m.insert(
        VolumeUp,
        vec![
            uc_press(
                "uc_volume_up_audio",
                LA::AUDIO_SYSTEM,
                Keycode::VOLUME_UP,
                VOL_HOLD,
            ),
            uc_press("uc_volume_up_tv", LA::TV, Keycode::VOLUME_UP, VOL_HOLD),
            uc_press(
                "uc_volume_up_playback1",
                LA::PLAYBACK_DEVICE_1,
                Keycode::VOLUME_UP,
                VOL_HOLD,
            ),
            libcec_volume("libcec_volume_up", StepKind::LibcecVolumeUp),
        ],
    );
    m.insert(
        VolumeDown,
        vec![
            uc_press(
                "uc_volume_down_audio",
                LA::AUDIO_SYSTEM,
                Keycode::VOLUME_DOWN,
                VOL_HOLD,
            ),
            uc_press("uc_volume_down_tv", LA::TV, Keycode::VOLUME_DOWN, VOL_HOLD),
            uc_press(
                "uc_volume_down_playback1",
                LA::PLAYBACK_DEVICE_1,
                Keycode::VOLUME_DOWN,
                VOL_HOLD,
            ),
            libcec_volume("libcec_volume_down", StepKind::LibcecVolumeDown),
        ],
    );
    m.insert(
        Mute,
        vec![
            uc_press("uc_mute_audio", LA::AUDIO_SYSTEM, Keycode::MUTE, VOL_HOLD),
            uc_press("uc_mute_tv", LA::TV, Keycode::MUTE, VOL_HOLD),
            libcec_volume("libcec_mute", StepKind::LibcecMute),
        ],
    );
    m.insert(
        NavUp,
        vec![uc_press("uc_up_target", LA::UNKNOWN, Keycode::UP, NAV_HOLD)],
    );
    m.insert(
        NavDown,
        vec![uc_press(
            "uc_down_target",
            LA::UNKNOWN,
            Keycode::DOWN,
            NAV_HOLD,
        )],
    );
    m.insert(
        NavLeft,
        vec![uc_press(
            "uc_left_target",
            LA::UNKNOWN,
            Keycode::LEFT,
            NAV_HOLD,
        )],
    );
    m.insert(
        NavRight,
        vec![uc_press(
            "uc_right_target",
            LA::UNKNOWN,
            Keycode::RIGHT,
            NAV_HOLD,
        )],
    );
    m.insert(
        Select,
        vec![uc_press(
            "uc_select_target",
            LA::UNKNOWN,
            Keycode::SELECT,
            NAV_HOLD,
        )],
    );
    m.insert(
        Back,
        vec![uc_press(
            "uc_back_target",
            LA::UNKNOWN,
            Keycode::EXIT,
            NAV_HOLD,
        )],
    );
    m.insert(
        Home,
        vec![uc_press(
            "uc_home_target",
            LA::UNKNOWN,
            Keycode::ROOT_MENU,
            NAV_HOLD,
        )],
    );
    m.insert(
        Menu,
        vec![uc_press(
            "uc_menu_target",
            LA::UNKNOWN,
            Keycode::SETUP_MENU,
            NAV_HOLD,
        )],
    );
    m.insert(
        ChannelUp,
        vec![
            uc_press(
                "uc_channel_up_tuner1",
                LA::TUNER_1,
                Keycode::CHANNEL_UP,
                NAV_HOLD,
            ),
            uc_press(
                "uc_channel_up_target",
                LA::UNKNOWN,
                Keycode::CHANNEL_UP,
                NAV_HOLD,
            ),
        ],
    );
    m.insert(
        ChannelDown,
        vec![
            uc_press(
                "uc_channel_down_tuner1",
                LA::TUNER_1,
                Keycode::CHANNEL_DOWN,
                NAV_HOLD,
            ),
            uc_press(
                "uc_channel_down_target",
                LA::UNKNOWN,
                Keycode::CHANNEL_DOWN,
                NAV_HOLD,
            ),
        ],
    );
    m.insert(
        Play,
        vec![uc_press("uc_play_target", LA::UNKNOWN, Keycode::PLAY, 0)],
    );
    m.insert(
        Pause,
        vec![uc_press("uc_pause_target", LA::UNKNOWN, Keycode::PAUSE, 0)],
    );
    m.insert(
        Stop,
        vec![uc_press("uc_stop_target", LA::UNKNOWN, Keycode::STOP, 0)],
    );
    m.insert(
        FastForward,
        vec![uc_press(
            "uc_ff_target",
            LA::UNKNOWN,
            Keycode::FAST_FORWARD,
            0,
        )],
    );
    m.insert(
        Rewind,
        vec![uc_press("uc_rew_target", LA::UNKNOWN, Keycode::REWIND, 0)],
    );
    m.insert(
        Record,
        vec![uc_press(
            "uc_record_target",
            LA::UNKNOWN,
            Keycode::RECORD,
            0,
        )],
    );
    m.insert(
        Power,
        vec![
            Strategy {
                name: "libcec_power_on_target".into(),
                steps: vec![Step {
                    kind: StepKind::LibcecPowerOn,
                    target: LA::UNKNOWN,
                    key: Keycode(0),
                    wait: false,
                    hold_ms: 0,
                    opcode: Opcode(0),
                    params: Vec::new(),
                    delay_ms: 0,
                }],
                observe_ms: 0,
            },
            uc_press("uc_power_target", LA::UNKNOWN, Keycode::POWER, NAV_HOLD),
        ],
    );
    for n in 0..=9u8 {
        let key = match n {
            0 => Keycode::K0,
            1 => Keycode::K1,
            2 => Keycode::K2,
            3 => Keycode::K3,
            4 => Keycode::K4,
            5 => Keycode::K5,
            6 => Keycode::K6,
            7 => Keycode::K7,
            8 => Keycode::K8,
            _ => Keycode::K9,
        };
        let action = match n {
            0 => Number(0),
            1 => Number(1),
            2 => Number(2),
            3 => Number(3),
            4 => Number(4),
            5 => Number(5),
            6 => Number(6),
            7 => Number(7),
            8 => Number(8),
            _ => Number(9),
        };
        m.insert(
            action,
            vec![uc_press(
                format!("uc_{n}_target"),
                LA::UNKNOWN,
                key,
                NAV_HOLD,
            )],
        );
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_parse_is_separator_insensitive() {
        assert_eq!(Action::parse("volume_up"), Some(Action::VolumeUp));
        assert_eq!(Action::parse("volume-up"), Some(Action::VolumeUp));
        assert_eq!(Action::parse("VolumeUp"), None); // canonical form is snake_case
        assert_eq!(Action::parse(" number_5 "), Some(Action::Number(5)));
        assert_eq!(Action::parse("nope"), None);
        assert_eq!(Action::VolumeUp.as_str(), "volume_up");
    }

    #[test]
    fn default_chains_cover_every_action() {
        let r = Registry::new();
        for (name, action) in ALL_ACTIONS {
            let chain = r.strategies_for("", *action);
            assert!(!chain.is_empty(), "{name} has defaults");
            for s in &chain {
                assert!(!s.steps.is_empty(), "{} strategy has steps", s.name);
            }
        }
    }

    #[test]
    fn vendor_overrides_take_precedence_and_clear() {
        let r = Registry::new();
        let action = Action::VolumeUp;
        let custom = vec![Strategy {
            name: "custom".into(),
            steps: vec![],
            observe_ms: 0,
        }];
        r.set_vendor_override("0x000048", action, custom.clone());
        assert_eq!(r.strategies_for("0X000048 ", action)[0].name, "custom");
        assert_eq!(
            r.strategies_for("0x0000ff", action)[0].name,
            "uc_volume_up_audio"
        );
        // Empty list clears the override back to defaults.
        r.set_vendor_override("0x000048", action, vec![]);
        assert_eq!(
            r.strategies_for("0x000048", action)[0].name,
            "uc_volume_up_audio"
        );
    }

    #[test]
    fn clamp_ms_bounds() {
        assert_eq!(clamp_ms(-5, 100), 0);
        assert_eq!(clamp_ms(50, 100), 50);
        assert_eq!(clamp_ms(5000, 100), 100);
    }

    #[test]
    fn classify_paths() {
        use crate::types::BusFrameEntry;
        let mut res = StratResult {
            strategy: "t".into(),
            status: StratStatus::AckedNoReply,
            acked: true,
            reply_opcode: 0,
            reply_name: String::new(),
            abort_opcode: 0,
            elapsed_ms: 0,
            error: String::new(),
            steps: vec![],
        };
        let own = 4;
        let frame = |op: &str, params: &[&str]| BusFrameEntry {
            timestamp: chrono::Utc::now(),
            initiator: 0,
            destination: 4,
            opcode: op.into(),
            ack: true,
            eom: true,
            opcode_set: true,
            params_hex: params.iter().map(|s| s.to_string()).collect(),
        };

        // Expected reply -> Ok (audio status replies with 0x7A)
        classify(
            &mut res,
            &[frame("0x7A", &[])],
            Some(Opcode::REPORT_AUDIO_STATUS),
            own,
        );
        assert_eq!(res.status, StratStatus::Ok);
        assert_eq!(res.reply_name, "REPORT_AUDIO_STATUS");

        // FeatureAbort referencing our opcode -> aborted
        classify(
            &mut res,
            &[frame("0x00", &["44", "01"])],
            Some(Opcode::REPORT_AUDIO_STATUS),
            own,
        );
        assert_eq!(res.status, StratStatus::FeatureAborted);
        assert_eq!(res.abort_opcode, 0x44);

        // Own frames are skipped
        let own_frame = BusFrameEntry {
            initiator: own,
            ..frame("0x90", &[])
        };
        classify(
            &mut res,
            &[own_frame],
            Some(Opcode::REPORT_AUDIO_STATUS),
            own,
        );
        assert_eq!(res.status, StratStatus::AckedNoReply);

        // No ack observed -> NoAck
        res.acked = false;
        classify(&mut res, &[], Some(Opcode::REPORT_AUDIO_STATUS), own);
        assert_eq!(res.status, StratStatus::NoAck);
    }

    #[test]
    fn parse_hex_byte_handles_prefixes_and_garbage() {
        assert_eq!(parse_hex_byte("0x90"), 0x90);
        assert_eq!(parse_hex_byte("0XFF"), 0xFF);
        assert_eq!(parse_hex_byte("90"), 0x90);
        assert_eq!(parse_hex_byte("zz"), 0);
    }

    #[test]
    fn expected_reply_opcode_mapping() {
        let step = |kind, key, op| Step {
            kind,
            target: LogicalAddress::UNKNOWN,
            key,
            wait: false,
            hold_ms: 0,
            opcode: op,
            params: vec![],
            delay_ms: 0,
        };
        assert_eq!(
            expected_reply_opcode(&step(
                StepKind::SendUserControl,
                Keycode::VOLUME_UP,
                Opcode(0)
            )),
            Some(Opcode::REPORT_AUDIO_STATUS)
        );
        assert_eq!(
            expected_reply_opcode(&step(StepKind::LibcecPowerOn, Keycode(0), Opcode(0))),
            Some(Opcode::REPORT_POWER_STATUS)
        );
        assert_eq!(
            expected_reply_opcode(&step(
                StepKind::Transmit,
                Keycode(0),
                Opcode::GIVE_PHYSICAL_ADDRESS
            )),
            Some(Opcode::REPORT_PHYSICAL_ADDRESS)
        );
        assert_eq!(
            expected_reply_opcode(&step(StepKind::Wait, Keycode(0), Opcode(0))),
            None
        );
    }
}
