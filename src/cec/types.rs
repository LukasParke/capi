//! Safe CEC domain types: port of `cec/types.go` and `cec/helpers.go`
//! (pure parts), plus the keycode/opcode name tables from
//! `capi/cec_exec.go` (`keyNameMap`) and `capi/strategies.go`
//! (`opcodeNames`). Display strings match the Go `String()` methods
//! EXACTLY — they appear in JSON wire output.

use std::fmt;

use serde_json::{Map, Number, Value};

use crate::cec::ffi::{CEC_INVALID_PHYSICAL_ADDRESS, CEC_MAX_DATA_PACKET_SIZE};

// ---------------------------------------------------------------------------
// LogicalAddress
// ---------------------------------------------------------------------------

/// A CEC logical address (0-15) or "unknown".
///
/// The CEC spec uses 0xF for both broadcast and "no/unknown" depending on
/// the field; libcec internally uses -1 (cast to 0xFF) for the "no address"
/// sentinel. We keep them distinct so callers can tell them apart.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LogicalAddress(pub u8);

impl LogicalAddress {
    pub const TV: LogicalAddress = LogicalAddress(0x00);
    pub const RECORDING_DEVICE_1: LogicalAddress = LogicalAddress(0x01);
    pub const RECORDING_DEVICE_2: LogicalAddress = LogicalAddress(0x02);
    pub const TUNER_1: LogicalAddress = LogicalAddress(0x03);
    pub const PLAYBACK_DEVICE_1: LogicalAddress = LogicalAddress(0x04);
    pub const AUDIO_SYSTEM: LogicalAddress = LogicalAddress(0x05);
    pub const TUNER_2: LogicalAddress = LogicalAddress(0x06);
    pub const TUNER_3: LogicalAddress = LogicalAddress(0x07);
    pub const PLAYBACK_DEVICE_2: LogicalAddress = LogicalAddress(0x08);
    pub const RECORDING_DEVICE_3: LogicalAddress = LogicalAddress(0x09);
    pub const TUNER_4: LogicalAddress = LogicalAddress(0x0A);
    pub const PLAYBACK_DEVICE_3: LogicalAddress = LogicalAddress(0x0B);
    pub const RESERVED_1: LogicalAddress = LogicalAddress(0x0C);
    pub const RESERVED_2: LogicalAddress = LogicalAddress(0x0D);
    pub const FREE_USE: LogicalAddress = LogicalAddress(0x0E);
    pub const BROADCAST: LogicalAddress = LogicalAddress(0x0F);
    pub const UNKNOWN: LogicalAddress = LogicalAddress(0xFF);

    /// A real CEC logical address (0-14). Broadcast (0xF) and Unknown (0xFF)
    /// are not valid device addresses.
    pub fn is_valid(self) -> bool {
        self.0 <= Self::FREE_USE.0
    }

    pub fn is_broadcast(self) -> bool {
        self == Self::BROADCAST
    }

    pub fn is_unknown(self) -> bool {
        self == Self::UNKNOWN
    }
}

/// Display name for a raw logical address byte; same strings as
/// [`LogicalAddress`]'s `Display` impl.
pub fn logical_address_name(addr: u8) -> String {
    LogicalAddress(addr).to_string()
}

impl fmt::Display for LogicalAddress {
    /// Matches Go `LogicalAddress.String()` exactly (JSON wire strings).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            Self::TV => "TV",
            Self::RECORDING_DEVICE_1 => "Recording Device 1",
            Self::RECORDING_DEVICE_2 => "Recording Device 2",
            Self::TUNER_1 => "Tuner 1",
            Self::PLAYBACK_DEVICE_1 => "Playback Device 1",
            Self::AUDIO_SYSTEM => "Audio System",
            Self::TUNER_2 => "Tuner 2",
            Self::TUNER_3 => "Tuner 3",
            Self::PLAYBACK_DEVICE_2 => "Playback Device 2",
            Self::RECORDING_DEVICE_3 => "Recording Device 3",
            Self::TUNER_4 => "Tuner 4",
            Self::PLAYBACK_DEVICE_3 => "Playback Device 3",
            Self::RESERVED_1 => "Reserved 1",
            Self::RESERVED_2 => "Reserved 2",
            Self::FREE_USE => "Free Use",
            Self::BROADCAST => "Broadcast",
            _ => "Unknown",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// DeviceType
// ---------------------------------------------------------------------------

/// A CEC device type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceType(pub u8);

impl DeviceType {
    pub const TV: DeviceType = DeviceType(0);
    pub const RECORDING: DeviceType = DeviceType(1);
    pub const RESERVED: DeviceType = DeviceType(2);
    pub const TUNER: DeviceType = DeviceType(3);
    pub const PLAYBACK: DeviceType = DeviceType(4);
    pub const AUDIO_SYSTEM: DeviceType = DeviceType(5);
}

impl fmt::Display for DeviceType {
    /// Matches Go `DeviceType.String()` exactly.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            Self::TV => "TV",
            Self::RECORDING => "Recording Device",
            Self::TUNER => "Tuner",
            Self::PLAYBACK => "Playback Device",
            Self::AUDIO_SYSTEM => "Audio System",
            _ => "Reserved",
        };
        f.write_str(s)
    }
}

/// Expected device type (as a compact role string) for a logical address.
/// Role strings are space-free variants used by the service core.
pub fn device_type_for_address(addr: u8) -> &'static str {
    match LogicalAddress(addr) {
        LogicalAddress::TV => "TV",
        LogicalAddress::RECORDING_DEVICE_1 => "RecordingDevice1",
        LogicalAddress::RECORDING_DEVICE_2 => "RecordingDevice2",
        LogicalAddress::RECORDING_DEVICE_3 => "RecordingDevice3",
        LogicalAddress::TUNER_1 => "Tuner1",
        LogicalAddress::TUNER_2 => "Tuner2",
        LogicalAddress::TUNER_3 => "Tuner3",
        LogicalAddress::TUNER_4 => "Tuner4",
        LogicalAddress::PLAYBACK_DEVICE_1 => "PlaybackDevice1",
        LogicalAddress::PLAYBACK_DEVICE_2 => "PlaybackDevice2",
        LogicalAddress::PLAYBACK_DEVICE_3 => "PlaybackDevice3",
        LogicalAddress::AUDIO_SYSTEM => "AudioSystem",
        LogicalAddress::FREE_USE => "FreeUse",
        LogicalAddress::BROADCAST => "Broadcast",
        _ => "Unknown",
    }
}

/// Port of Go `DeviceTypeForAddress`.
pub fn expected_device_type(addr: u8) -> DeviceType {
    match LogicalAddress(addr) {
        LogicalAddress::TV => DeviceType::TV,
        LogicalAddress::RECORDING_DEVICE_1
        | LogicalAddress::RECORDING_DEVICE_2
        | LogicalAddress::RECORDING_DEVICE_3 => DeviceType::RECORDING,
        LogicalAddress::TUNER_1
        | LogicalAddress::TUNER_2
        | LogicalAddress::TUNER_3
        | LogicalAddress::TUNER_4 => DeviceType::TUNER,
        LogicalAddress::PLAYBACK_DEVICE_1
        | LogicalAddress::PLAYBACK_DEVICE_2
        | LogicalAddress::PLAYBACK_DEVICE_3 => DeviceType::PLAYBACK,
        LogicalAddress::AUDIO_SYSTEM => DeviceType::AUDIO_SYSTEM,
        _ => DeviceType::RESERVED,
    }
}

// ---------------------------------------------------------------------------
// PowerStatus
// ---------------------------------------------------------------------------

/// Device power status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PowerStatus(pub u8);

impl PowerStatus {
    pub const ON: PowerStatus = PowerStatus(0x00);
    pub const STANDBY: PowerStatus = PowerStatus(0x01);
    pub const IN_TRANSITION_STANDBY_TO_ON: PowerStatus = PowerStatus(0x02);
    pub const IN_TRANSITION_ON_TO_STANDBY: PowerStatus = PowerStatus(0x03);
    /// Go-side sentinel (0xFF). Distinct from libcec's wire sentinel 0x99
    /// (`CEC_POWER_STATUS_UNKNOWN`), which `get_device_power_status` maps
    /// to an error — the Go sentinel mismatch is preserved on purpose.
    pub const UNKNOWN: PowerStatus = PowerStatus(0xFF);
}

impl fmt::Display for PowerStatus {
    /// Matches Go `PowerStatus.String()` exactly.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            Self::ON => "On",
            Self::STANDBY => "Standby",
            Self::IN_TRANSITION_STANDBY_TO_ON => "Transitioning to On",
            Self::IN_TRANSITION_ON_TO_STANDBY => "Transitioning to Standby",
            _ => "Unknown",
        };
        f.write_str(s)
    }
}

/// Maps a raw CEC power status byte to the wire string used by
/// Go `powerStatusFromByte` (capi/cec_events.go).
pub fn power_status_str(raw: u8) -> String {
    match raw {
        0x00 => "on",
        0x01 => "standby",
        0x02 => "transitioning_to_on",
        0x03 => "transitioning_to_standby",
        _ => "unknown",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// CECVersion
// ---------------------------------------------------------------------------

/// CEC spec version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CECVersion(pub u8);

impl CECVersion {
    pub const UNKNOWN: CECVersion = CECVersion(0x00);
    pub const V1_2: CECVersion = CECVersion(0x01);
    pub const V1_2A: CECVersion = CECVersion(0x02);
    pub const V1_3: CECVersion = CECVersion(0x03);
    pub const V1_3A: CECVersion = CECVersion(0x04);
    pub const V1_4: CECVersion = CECVersion(0x05);
}

impl fmt::Display for CECVersion {
    /// Matches Go `CECVersion.String()` exactly.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            Self::V1_2 => "1.2",
            Self::V1_2A => "1.2a",
            Self::V1_3 => "1.3",
            Self::V1_3A => "1.3a",
            Self::V1_4 => "1.4",
            _ => "Unknown",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// Opcode
// ---------------------------------------------------------------------------

/// A CEC opcode.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Opcode(pub u8);

impl Opcode {
    pub const ACTIVE_SOURCE: Opcode = Opcode(0x82);
    pub const IMAGE_VIEW_ON: Opcode = Opcode(0x04);
    pub const TEXT_VIEW_ON: Opcode = Opcode(0x0D);
    pub const INACTIVE_SOURCE: Opcode = Opcode(0x9D);
    pub const REQUEST_ACTIVE_SOURCE: Opcode = Opcode(0x85);
    pub const ROUTING_CHANGE: Opcode = Opcode(0x80);
    pub const ROUTING_INFORMATION: Opcode = Opcode(0x81);
    pub const SET_STREAM_PATH: Opcode = Opcode(0x86);
    pub const STANDBY: Opcode = Opcode(0x36);
    pub const RECORD_OFF: Opcode = Opcode(0x0B);
    pub const RECORD_ON: Opcode = Opcode(0x09);
    pub const RECORD_STATUS: Opcode = Opcode(0x0A);
    pub const RECORD_TV_SCREEN: Opcode = Opcode(0x0F);
    pub const CLEAR_ANALOGUE_TIMER: Opcode = Opcode(0x33);
    pub const CLEAR_DIGITAL_TIMER: Opcode = Opcode(0x99);
    pub const CLEAR_EXTERNAL_TIMER: Opcode = Opcode(0xA1);
    pub const SET_ANALOGUE_TIMER: Opcode = Opcode(0x34);
    pub const SET_DIGITAL_TIMER: Opcode = Opcode(0x97);
    pub const SET_EXTERNAL_TIMER: Opcode = Opcode(0xA2);
    pub const SET_TIMER_PROGRAM_TITLE: Opcode = Opcode(0x67);
    pub const TIMER_CLEARED_STATUS: Opcode = Opcode(0x43);
    pub const TIMER_STATUS: Opcode = Opcode(0x35);
    pub const CEC_VERSION: Opcode = Opcode(0x9E);
    pub const GET_CEC_VERSION: Opcode = Opcode(0x9F);
    pub const GIVE_PHYSICAL_ADDRESS: Opcode = Opcode(0x83);
    pub const GET_MENU_LANGUAGE: Opcode = Opcode(0x91);
    pub const REPORT_PHYSICAL_ADDRESS: Opcode = Opcode(0x84);
    pub const SET_MENU_LANGUAGE: Opcode = Opcode(0x32);
    pub const DECK_CONTROL: Opcode = Opcode(0x42);
    pub const DECK_STATUS: Opcode = Opcode(0x1B);
    pub const GIVE_DECK_STATUS: Opcode = Opcode(0x1A);
    pub const PLAY: Opcode = Opcode(0x41);
    pub const GIVE_TUNER_DEVICE_STATUS: Opcode = Opcode(0x08);
    pub const SELECT_ANALOGUE_SERVICE: Opcode = Opcode(0x92);
    pub const SELECT_DIGITAL_SERVICE: Opcode = Opcode(0x93);
    pub const TUNER_DEVICE_STATUS: Opcode = Opcode(0x07);
    pub const TUNER_STEP_DECREMENT: Opcode = Opcode(0x06);
    pub const TUNER_STEP_INCREMENT: Opcode = Opcode(0x05);
    pub const DEVICE_VENDOR_ID: Opcode = Opcode(0x87);
    pub const GIVE_DEVICE_VENDOR_ID: Opcode = Opcode(0x8C);
    pub const VENDOR_COMMAND: Opcode = Opcode(0x89);
    pub const VENDOR_COMMAND_WITH_ID: Opcode = Opcode(0xA0);
    pub const VENDOR_REMOTE_BUTTON_DOWN: Opcode = Opcode(0x8A);
    pub const VENDOR_REMOTE_BUTTON_UP: Opcode = Opcode(0x8B);
    pub const SET_OSD_STRING: Opcode = Opcode(0x64);
    pub const GIVE_OSD_NAME: Opcode = Opcode(0x46);
    pub const SET_OSD_NAME: Opcode = Opcode(0x47);
    pub const MENU_REQUEST: Opcode = Opcode(0x8D);
    pub const MENU_STATUS: Opcode = Opcode(0x8E);
    pub const USER_CONTROL_PRESSED: Opcode = Opcode(0x44);
    pub const USER_CONTROL_RELEASED: Opcode = Opcode(0x45);
    pub const GIVE_DEVICE_POWER_STATUS: Opcode = Opcode(0x8F);
    pub const REPORT_POWER_STATUS: Opcode = Opcode(0x90);
    pub const FEATURE_ABORT: Opcode = Opcode(0x00);
    pub const ABORT: Opcode = Opcode(0xFF);
    pub const GIVE_AUDIO_STATUS: Opcode = Opcode(0x71);
    pub const GIVE_SYSTEM_AUDIO_MODE_STATUS: Opcode = Opcode(0x7D);
    pub const REPORT_AUDIO_STATUS: Opcode = Opcode(0x7A);
    pub const SET_SYSTEM_AUDIO_MODE: Opcode = Opcode(0x72);
    pub const SYSTEM_AUDIO_MODE_REQUEST: Opcode = Opcode(0x70);
    pub const SYSTEM_AUDIO_MODE_STATUS: Opcode = Opcode(0x7E);
    pub const SET_AUDIO_RATE: Opcode = Opcode(0x9A);
}

/// Name table ported from Go `opcodeNames` (capi/strategies.go); first
/// match wins, unknown opcodes format as uppercase hex like Go `opcodeName`.
pub const OPCODE_NAMES: &[(Opcode, &str)] = &[
    (Opcode::REPORT_POWER_STATUS, "REPORT_POWER_STATUS"),
    (Opcode::REPORT_AUDIO_STATUS, "REPORT_AUDIO_STATUS"),
    (Opcode::REPORT_PHYSICAL_ADDRESS, "REPORT_PHYSICAL_ADDRESS"),
    (Opcode::DEVICE_VENDOR_ID, "DEVICE_VENDOR_ID"),
    (Opcode::SET_OSD_NAME, "SET_OSD_NAME"),
    (Opcode::CEC_VERSION, "CEC_VERSION"),
    (Opcode::FEATURE_ABORT, "FEATURE_ABORT"),
    (Opcode::ACTIVE_SOURCE, "ACTIVE_SOURCE"),
    (Opcode::ROUTING_CHANGE, "ROUTING_CHANGE"),
    (Opcode::ROUTING_INFORMATION, "ROUTING_INFORMATION"),
    (Opcode::SET_STREAM_PATH, "SET_STREAM_PATH"),
    (Opcode::MENU_STATUS, "MENU_STATUS"),
    (Opcode::SYSTEM_AUDIO_MODE_STATUS, "SYSTEM_AUDIO_MODE_STATUS"),
    (Opcode::STANDBY, "STANDBY"),
    (Opcode::IMAGE_VIEW_ON, "IMAGE_VIEW_ON"),
    (Opcode::TEXT_VIEW_ON, "TEXT_VIEW_ON"),
    (Opcode::USER_CONTROL_PRESSED, "USER_CONTROL_PRESSED"),
    (Opcode::USER_CONTROL_RELEASED, "USER_CONTROL_RELEASED"),
];

/// Human-readable opcode name; falls back to hex like Go `opcodeName`.
pub fn opcode_name(op: Opcode) -> String {
    for (o, name) in OPCODE_NAMES {
        if *o == op {
            return (*name).to_string();
        }
    }
    format!("0x{:02X}", op.0)
}

impl Opcode {
    pub fn name(self) -> String {
        opcode_name(self)
    }
}

// ---------------------------------------------------------------------------
// Keycode
// ---------------------------------------------------------------------------

/// A CEC user control code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Keycode(pub u8);

impl Keycode {
    pub const SELECT: Keycode = Keycode(0x00);
    pub const UP: Keycode = Keycode(0x01);
    pub const DOWN: Keycode = Keycode(0x02);
    pub const LEFT: Keycode = Keycode(0x03);
    pub const RIGHT: Keycode = Keycode(0x04);
    pub const RIGHT_UP: Keycode = Keycode(0x05);
    pub const RIGHT_DOWN: Keycode = Keycode(0x06);
    pub const LEFT_UP: Keycode = Keycode(0x07);
    pub const LEFT_DOWN: Keycode = Keycode(0x08);
    pub const ROOT_MENU: Keycode = Keycode(0x09);
    pub const SETUP_MENU: Keycode = Keycode(0x0A);
    pub const CONTENTS_MENU: Keycode = Keycode(0x0B);
    pub const FAVORITE_MENU: Keycode = Keycode(0x0C);
    pub const EXIT: Keycode = Keycode(0x0D);
    pub const K0: Keycode = Keycode(0x20);
    pub const K1: Keycode = Keycode(0x21);
    pub const K2: Keycode = Keycode(0x22);
    pub const K3: Keycode = Keycode(0x23);
    pub const K4: Keycode = Keycode(0x24);
    pub const K5: Keycode = Keycode(0x25);
    pub const K6: Keycode = Keycode(0x26);
    pub const K7: Keycode = Keycode(0x27);
    pub const K8: Keycode = Keycode(0x28);
    pub const K9: Keycode = Keycode(0x29);
    pub const DOT: Keycode = Keycode(0x2A);
    pub const ENTER: Keycode = Keycode(0x2B);
    pub const CLEAR: Keycode = Keycode(0x2C);
    pub const CHANNEL_UP: Keycode = Keycode(0x30);
    pub const CHANNEL_DOWN: Keycode = Keycode(0x31);
    pub const PREVIOUS_CHANNEL: Keycode = Keycode(0x32);
    pub const SOUND_SELECT: Keycode = Keycode(0x33);
    pub const INPUT_SELECT: Keycode = Keycode(0x34);
    pub const DISPLAY_INFORMATION: Keycode = Keycode(0x35);
    pub const HELP: Keycode = Keycode(0x36);
    pub const PAGE_UP: Keycode = Keycode(0x37);
    pub const PAGE_DOWN: Keycode = Keycode(0x38);
    pub const POWER: Keycode = Keycode(0x40);
    pub const VOLUME_UP: Keycode = Keycode(0x41);
    pub const VOLUME_DOWN: Keycode = Keycode(0x42);
    pub const MUTE: Keycode = Keycode(0x43);
    pub const PLAY: Keycode = Keycode(0x44);
    pub const STOP: Keycode = Keycode(0x45);
    pub const PAUSE: Keycode = Keycode(0x46);
    pub const RECORD: Keycode = Keycode(0x47);
    pub const REWIND: Keycode = Keycode(0x48);
    pub const FAST_FORWARD: Keycode = Keycode(0x49);
    pub const EJECT: Keycode = Keycode(0x4A);
    pub const FORWARD: Keycode = Keycode(0x4B);
    pub const BACKWARD: Keycode = Keycode(0x4C);
    pub const ANGLE: Keycode = Keycode(0x50);
    pub const SUBPICTURE: Keycode = Keycode(0x51);
    pub const F1_BLUE: Keycode = Keycode(0x71);
    pub const F2_RED: Keycode = Keycode(0x72);
    pub const F3_GREEN: Keycode = Keycode(0x73);
    pub const F4_YELLOW: Keycode = Keycode(0x74);
    pub const F5: Keycode = Keycode(0x75);
}

/// Canonical lowercase-underscore name table ported from Go `keyNameMap`
/// (capi/cec_exec.go), including the aliases (`home`→RootMenu,
/// `back`→Exit, `menu`→SetupMenu). Multiple names may map to one keycode;
/// the FIRST name for a code is its canonical name.
pub const KEY_NAMES: &[(&str, Keycode)] = &[
    // Navigation
    ("select", Keycode::SELECT),
    ("up", Keycode::UP),
    ("down", Keycode::DOWN),
    ("left", Keycode::LEFT),
    ("right", Keycode::RIGHT),
    ("right_up", Keycode::RIGHT_UP),
    ("right_down", Keycode::RIGHT_DOWN),
    ("left_up", Keycode::LEFT_UP),
    ("left_down", Keycode::LEFT_DOWN),
    ("root_menu", Keycode::ROOT_MENU),
    ("home", Keycode::ROOT_MENU),
    ("setup_menu", Keycode::SETUP_MENU),
    ("menu", Keycode::SETUP_MENU),
    ("contents_menu", Keycode::CONTENTS_MENU),
    ("favorite_menu", Keycode::FAVORITE_MENU),
    ("exit", Keycode::EXIT),
    ("back", Keycode::EXIT),
    ("enter", Keycode::ENTER),
    ("clear", Keycode::CLEAR),
    // Number pad
    ("0", Keycode::K0),
    ("1", Keycode::K1),
    ("2", Keycode::K2),
    ("3", Keycode::K3),
    ("4", Keycode::K4),
    ("5", Keycode::K5),
    ("6", Keycode::K6),
    ("7", Keycode::K7),
    ("8", Keycode::K8),
    ("9", Keycode::K9),
    ("dot", Keycode::DOT),
    // Channels / inputs
    ("channel_up", Keycode::CHANNEL_UP),
    ("channel_down", Keycode::CHANNEL_DOWN),
    ("previous_channel", Keycode::PREVIOUS_CHANNEL),
    ("sound_select", Keycode::SOUND_SELECT),
    ("input_select", Keycode::INPUT_SELECT),
    ("display_information", Keycode::DISPLAY_INFORMATION),
    ("help", Keycode::HELP),
    ("page_up", Keycode::PAGE_UP),
    ("page_down", Keycode::PAGE_DOWN),
    // Power / volume
    ("power", Keycode::POWER),
    ("volume_up", Keycode::VOLUME_UP),
    ("volume_down", Keycode::VOLUME_DOWN),
    ("mute", Keycode::MUTE),
    // Transport
    ("play", Keycode::PLAY),
    ("stop", Keycode::STOP),
    ("pause", Keycode::PAUSE),
    ("record", Keycode::RECORD),
    ("rewind", Keycode::REWIND),
    ("fast_forward", Keycode::FAST_FORWARD),
    ("eject", Keycode::EJECT),
    ("forward", Keycode::FORWARD),
    ("backward", Keycode::BACKWARD),
    ("angle", Keycode::ANGLE),
    ("subpicture", Keycode::SUBPICTURE),
    // Coloured buttons
    ("f1_blue", Keycode::F1_BLUE),
    ("f2_red", Keycode::F2_RED),
    ("f3_green", Keycode::F3_GREEN),
    ("f4_yellow", Keycode::F4_YELLOW),
    ("f5", Keycode::F5),
];

/// Resolves a canonical key name (see [`KEY_NAMES`]) to a keycode.
pub fn keycode_from_name(name: &str) -> Option<Keycode> {
    KEY_NAMES.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}

impl Keycode {
    pub fn from_name(name: &str) -> Option<Keycode> {
        keycode_from_name(name)
    }

    /// Canonical name for this keycode (first entry in [`KEY_NAMES`] with
    /// this code), if any.
    pub fn canonical_name(self) -> Option<&'static str> {
        KEY_NAMES.iter().find(|(_, k)| *k == self).map(|(n, _)| *n)
    }
}

// ---------------------------------------------------------------------------
// Small enums
// ---------------------------------------------------------------------------

/// OSD display duration control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplayControl(pub u8);

impl DisplayControl {
    pub const DEFAULT_TIME: DisplayControl = DisplayControl(0x00);
    pub const UNTIL_CLEARED: DisplayControl = DisplayControl(0x40);
    pub const CLEAR_PREVIOUS: DisplayControl = DisplayControl(0x80);
    pub const RESERVED: DisplayControl = DisplayControl(0xC0);
}

/// Menu state reported by libcec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MenuState(pub u8);

impl MenuState {
    pub const ACTIVATED: MenuState = MenuState(0x00);
    pub const DEACTIVATED: MenuState = MenuState(0x01);
}

/// libcec log message level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LogLevel(pub i32);

impl LogLevel {
    pub const ERROR: LogLevel = LogLevel(1);
    pub const WARNING: LogLevel = LogLevel(2);
    pub const NOTICE: LogLevel = LogLevel(4);
    pub const TRAFFIC: LogLevel = LogLevel(8);
    pub const DEBUG: LogLevel = LogLevel(16);
    pub const ALL: LogLevel = LogLevel(31);
}

impl fmt::Display for LogLevel {
    /// Matches Go `LogLevel.String()` exactly.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            Self::ERROR => "ERROR",
            Self::WARNING => "WARNING",
            Self::NOTICE => "NOTICE",
            Self::TRAFFIC => "TRAFFIC",
            Self::DEBUG => "DEBUG",
            _ => "ALL",
        };
        f.write_str(s)
    }
}

/// libcec alert type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Alert(pub i32);

impl Alert {
    pub const SERVICE_DEVICE: Alert = Alert(1);
    pub const CONNECTION_LOST: Alert = Alert(2);
    pub const PERMISSION_ERROR: Alert = Alert(3);
    pub const PORT_BUSY: Alert = Alert(4);
    pub const PHYSICAL_ADDRESS_ERROR: Alert = Alert(5);
    pub const TV_POLL_FAILED: Alert = Alert(6);
}

impl fmt::Display for Alert {
    /// Matches Go `Alert.String()` exactly.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            Self::SERVICE_DEVICE => "ServiceDevice",
            Self::CONNECTION_LOST => "ConnectionLost",
            Self::PERMISSION_ERROR => "PermissionError",
            Self::PORT_BUSY => "PortBusy",
            Self::PHYSICAL_ADDRESS_ERROR => "PhysicalAddressError",
            Self::TV_POLL_FAILED => "TVPollFailed",
            _ => "Unknown",
        };
        f.write_str(s)
    }
}

/// An alert parameter (libcec_parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parameter {
    pub param_type: i32,
    pub value: i64,
}

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

/// A CEC command frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Command {
    pub initiator: LogicalAddress,
    pub destination: LogicalAddress,
    pub opcode: Opcode,
    pub opcode_set: bool,
    pub parameters: Vec<u8>,
    pub ack: bool,
    pub eom: bool,
}

/// A discoverable CEC adapter (USB or built-in).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub path: String,
    pub comm: String,
}

/// Session configuration, mirroring the Go `Configuration` struct subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Configuration {
    /// <=13 bytes; truncated by `sanitize_device_name` before it reaches
    /// libcec.
    pub device_name: String,
    pub device_type: DeviceType,
    /// 0xFFFF = auto-detect.
    pub physical_address: u16,
    pub base_device: LogicalAddress,
    pub hdmi_port: u8,
    /// When true, libcec does not allocate a logical address and transmits
    /// are refused.
    pub monitor_only: bool,
    /// When true, libcec announces itself as the active source on open.
    pub activate_source: bool,
    /// Logical addresses libcec wakes on connect (suppresses libcec's
    /// default of {TV} when empty).
    pub wake_devices: Vec<u8>,
    /// Logical addresses libcec puts in standby on disconnect (suppresses
    /// libcec's default of {BROADCAST} when empty).
    pub power_off_devices: Vec<u8>,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            device_name: String::new(),
            device_type: DeviceType::PLAYBACK,
            physical_address: 0xFFFF,
            base_device: LogicalAddress::UNKNOWN,
            hdmi_port: 0,
            monitor_only: false,
            activate_source: false,
            wake_devices: Vec::new(),
            power_off_devices: Vec::new(),
        }
    }
}

/// Truncates a device name to at most 13 bytes on a UTF-8 char boundary
/// and strips interior NULs so it can always become a C string (contract:
/// "<=13 chars, truncate").
pub fn sanitize_device_name(name: &str) -> String {
    let mut out = String::new();
    let mut len = 0usize;
    for c in name.chars() {
        if c == '\0' {
            break;
        }
        let cb = c.len_utf8();
        if len + cb > 13 {
            break;
        }
        out.push(c);
        len += cb;
    }
    out
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors returned by the cec module. Port of the Go sentinel errors; the
/// generic `ErrLibcecCall` carries the failing call as context.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CecError {
    #[error("cec: connection closed")]
    Closed,
    #[error("cec: connection is monitor-only; cannot transmit")]
    MonitorOnly,
    #[error("cec: invalid logical address")]
    InvalidLogicalAddress,
    #[error("cec: invalid HDMI port")]
    InvalidHdmiPort,
    #[error("cec: invalid parameters: {0}")]
    InvalidParams(String),
    #[error("cec: no active source")]
    NoActiveSource,
    #[error("cec: adapter not open")]
    AdapterNotOpen,
    #[error("cec: libcec call failed: {0}")]
    LibcecCall(String),
}

// ---------------------------------------------------------------------------
// Events (broadcast channel payloads)
// ---------------------------------------------------------------------------

/// Asynchronous event delivered on the connection's broadcast channel.
/// Menu state changes are ALSO delivered synchronously to the menu handler
/// (see `Connection::set_menu_state_handler`).
#[derive(Debug, Clone)]
pub enum CecEvent {
    Log {
        level: LogLevel,
        time: i64,
        message: String,
    },
    KeyPress {
        key: u8,
        duration: u32,
    },
    /// Fully copied out of the C struct before dispatch.
    Command(Command),
    ConfigurationChanged(Configuration),
    Alert {
        alert: i32,
        param_type: i32,
        param_value: i64,
    },
    MenuState {
        state: i32,
    },
    SourceActivated {
        address: u8,
        activated: bool,
    },
}

// ---------------------------------------------------------------------------
// Device info
// ---------------------------------------------------------------------------

/// Per-field failure flags for `get_device_info`, port of Go
/// `DeviceInfoErrors`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceInfoErrors {
    pub physical_address: bool,
    pub vendor_id: bool,
    pub cec_version: bool,
    pub power_status: bool,
    pub osd_name: bool,
    pub menu_language: bool,
}

impl DeviceInfoErrors {
    /// At least one sub-query failed.
    pub fn any(&self) -> bool {
        self.physical_address
            || self.vendor_id
            || self.cec_version
            || self.power_status
            || self.osd_name
            || self.menu_language
    }

    /// Every sub-query failed (device unresponsive).
    pub fn all(&self) -> bool {
        self.physical_address
            && self.vendor_id
            && self.cec_version
            && self.power_status
            && self.osd_name
            && self.menu_language
    }
}

/// Aggregated device information, port of Go `GetDeviceInfo` +
/// `deviceToMap`. Always constructible; failed sub-queries keep zero values
/// and are flagged in [`DeviceInfoErrors`].
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    pub logical_address: LogicalAddress,
    /// Display string of the logical address.
    pub address_name: String,
    pub physical_address: u16,
    pub osd_name: String,
    pub menu_language: String,
    pub vendor_id: u64,
    pub vendor_name: String,
    pub vendor_known: bool,
    pub cec_version: CECVersion,
    pub power_status: PowerStatus,
    /// HDMI port derived from the physical address (0 if not applicable).
    pub hdmi_port: u8,
    pub is_active: bool,
    pub is_active_source: bool,
    /// Which sub-queries failed while aggregating.
    pub errors: DeviceInfoErrors,
}

impl DeviceInfo {
    /// HDMI port derived like Go `deviceToMap`: first nibble of the
    /// physical address, unless the address is 0 or invalid.
    pub fn derive_hdmi_port(physical_address: u16) -> u8 {
        if physical_address != 0 && physical_address != CEC_INVALID_PHYSICAL_ADDRESS {
            ((physical_address >> 12) & 0xF) as u8
        } else {
            0
        }
    }

    /// JSON object with the same keys and formats as Go `deviceToMap`.
    pub fn to_map(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert(
            "logical_address".into(),
            Value::Number(Number::from(self.logical_address.0)),
        );
        m.insert(
            "address_name".into(),
            Value::String(self.address_name.clone()),
        );
        m.insert(
            "physical_address".into(),
            Value::String(physical_address_to_string(self.physical_address)),
        );
        m.insert("osd_name".into(), Value::String(self.osd_name.clone()));
        m.insert(
            "menu_language".into(),
            Value::String(self.menu_language.clone()),
        );
        m.insert(
            "vendor_id".into(),
            Value::String(format!("0x{:06x}", self.vendor_id)),
        );
        m.insert(
            "vendor_name".into(),
            Value::String(self.vendor_name.clone()),
        );
        m.insert("vendor_known".into(), Value::Bool(self.vendor_known));
        m.insert(
            "cec_version".into(),
            Value::String(self.cec_version.to_string()),
        );
        m.insert(
            "power_status".into(),
            Value::String(self.power_status.to_string()),
        );
        m.insert(
            "hdmi_port".into(),
            Value::Number(Number::from(self.hdmi_port)),
        );
        m.insert("is_active".into(), Value::Bool(self.is_active));
        m.insert(
            "is_active_source".into(),
            Value::Bool(self.is_active_source),
        );
        m
    }
}

// ---------------------------------------------------------------------------
// Vendor table + physical address helpers (cec/helpers.go)
// ---------------------------------------------------------------------------

/// Static vendor-ID -> human-name lookup (Go `vendorNames`).
pub const VENDOR_NAMES: &[(u64, &str)] = &[
    (0x000039, "Toshiba"),
    (0x000048, "LG"), // observed on LG projectors; OUI variant of 0x009053
    (0x0000F0, "Samsung"),
    (0x0005CD, "Denon"),
    (0x000678, "Marantz"),
    (0x000982, "Loewe"),
    (0x0009B0, "Onkyo"),
    (0x000CB8, "Medion"),
    (0x000CE7, "Toshiba"),
    (0x001582, "Pulse Eight"),
    (0x001950, "Google"),
    (0x001A11, "Akai"),
    (0x0020C7, "AOC"),
    (0x002467, "Panasonic"),
    (0x008045, "Philips"),
    (0x00903E, "Pioneer"),
    (0x009053, "LG"),
    (0x00A0DE, "Sharp"),
    (0x00D0D5, "Vizio"),
    (0x00E036, "Harman Kardon"),
    (0x00E091, "Yamaha"),
    (0x08001F, "Sony"),
    (0x18C086, "Broadcom"),
    (0x6B746D, "Vizio"),
    (0x8065E9, "Benq"),
    (0x9C645E, "Daewoo"),
];

/// Human-readable vendor name; unknown IDs format as `Unknown (0xABCDEF)`
/// like Go `GetVendorName`.
pub fn get_vendor_name(vendor_id: u64) -> String {
    match lookup_vendor(vendor_id) {
        Some(name) => name.to_string(),
        None => format!("Unknown (0x{vendor_id:06X})"),
    }
}

pub fn lookup_vendor(vendor_id: u64) -> Option<&'static str> {
    VENDOR_NAMES
        .iter()
        .find(|(id, _)| *id == vendor_id)
        .map(|(_, name)| *name)
}

/// Whether the vendor ID is in our table (Go `IsKnownVendor`).
pub fn is_known_vendor(vendor_id: u64) -> bool {
    lookup_vendor(vendor_id).is_some()
}

/// Converts a packed physical address into dotted form (0x2100 -> "2.1.0.0").
pub fn physical_address_to_string(addr: u16) -> String {
    format!(
        "{}.{}.{}.{}",
        (addr >> 12) & 0xF,
        (addr >> 8) & 0xF,
        (addr >> 4) & 0xF,
        addr & 0xF
    )
}

/// Parses dotted form back into the packed u16 (Go `ParsePhysicalAddress`).
pub fn parse_physical_address(addr: &str) -> Result<u16, CecError> {
    let parts: Vec<&str> = addr.split('.').collect();
    if parts.len() != 4 {
        return Err(CecError::InvalidParams(format!(
            "physical address {addr:?} must have 4 dotted components"
        )));
    }
    let mut out: u16 = 0;
    for part in parts {
        let v: u16 = part.parse().map_err(|_| {
            CecError::InvalidParams(format!("physical address {addr:?}: bad component {part:?}"))
        })?;
        if v > 15 {
            return Err(CecError::InvalidParams(format!(
                "physical address components must be 0-15 (got {v} in {addr:?})"
            )));
        }
        out = (out << 4) | v;
    }
    Ok(out)
}

/// Validates a command for transmit without needing a live session.
///
/// Bounds checks (fixing the Go bug): parameters longer than
/// `CEC_MAX_DATA_PACKET_SIZE` (64) are rejected, as is anything that would
/// truncate the u8 size field (> 255). Initiator/destination must be valid
/// logical addresses (0..=15; 0xFF "unknown" is rejected).
pub fn validate_transmit(cmd: &Command) -> Result<(), CecError> {
    if cmd.initiator.0 > 15 {
        return Err(CecError::InvalidParams(format!(
            "initiator {} is not a valid logical address (0..=15)",
            cmd.initiator.0
        )));
    }
    if cmd.destination.0 > 15 {
        return Err(CecError::InvalidParams(format!(
            "destination {} is not a valid logical address (0..=15)",
            cmd.destination.0
        )));
    }
    if cmd.parameters.len() > CEC_MAX_DATA_PACKET_SIZE {
        return Err(CecError::InvalidParams(format!(
            "parameters.len() = {} exceeds CEC_MAX_DATA_PACKET_SIZE ({CEC_MAX_DATA_PACKET_SIZE})",
            cmd.parameters.len()
        )));
    }
    if cmd.parameters.len() > u8::MAX as usize {
        return Err(CecError::InvalidParams(format!(
            "parameters.len() = {} would truncate the u8 size field",
            cmd.parameters.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod table_sweeps {
    use super::*;

    #[test]
    fn logical_address_names_cover_full_byte_range() {
        for b in 0u8..=255 {
            let name = logical_address_name(b);
            assert!(!name.is_empty(), "{b}");
        }
        assert_eq!(logical_address_name(0), "TV");
        assert_eq!(logical_address_name(14), "Free Use");
        assert_eq!(logical_address_name(15), "Broadcast");
    }

    #[test]
    fn device_type_roles_cover_all_addresses() {
        for b in 0u8..=15 {
            assert!(!device_type_for_address(b).is_empty(), "{b}");
        }
    }

    #[test]
    fn power_status_str_covers_common_bytes() {
        for b in [0x00, 0x01, 0x02, 0x03, 0x99, 0xFF] {
            assert!(!power_status_str(b).is_empty(), "{b:#x}");
        }
    }

    #[test]
    fn opcode_table_round_trips_through_opcode_name() {
        let table = crate::cec::opcode_table();
        assert!(!table.is_empty());
        for (name, code) in &table {
            let got = opcode_name(Opcode(*code));
            assert_eq!(&got, name, "opcode {code:#x}");
        }
    }

    #[test]
    fn keycode_table_round_trips_through_keycode_from_name() {
        let table = crate::cec::keycode_names();
        assert!(table.len() >= 50);
        for (idx, (name, code)) in table.iter().enumerate() {
            if idx > 0 {
                let (_, prev_code) = &table[idx - 1];
                assert!(prev_code <= code, "table sorted by code");
            }
            assert_eq!(keycode_from_name(name).unwrap().0, *code, "{name}");
        }
        // Canonical names used across HTTP/UI/MQTT.
        for n in [
            "select",
            "up",
            "down",
            "left",
            "right",
            "exit",
            "root_menu",
            "setup_menu",
            "volume_up",
            "volume_down",
            "mute",
            "play",
            "pause",
            "stop",
            "fast_forward",
            "rewind",
            "record",
            "channel_up",
            "channel_down",
            "power",
            "f1_blue",
        ] {
            assert!(keycode_from_name(n).is_some(), "{n}");
        }
        assert!(keycode_from_name("not-a-key").is_none());
    }

    #[test]
    fn display_impls_match_free_functions() {
        for b in 0u8..=255 {
            assert_eq!(LogicalAddress(b).to_string(), logical_address_name(b));
        }
        assert_eq!(
            PowerStatus::ON.to_string().to_lowercase(),
            power_status_str(0x00).to_lowercase()
        );
        assert_eq!(DeviceType::TV.to_string(), "TV");
        assert_eq!(CECVersion::V1_4.to_string(), "1.4");
    }
}

#[cfg(test)]
mod display_sweeps {
    use super::*;

    #[test]
    fn device_type_display_covers_all_variants() {
        for b in 0u8..=255 {
            let s = DeviceType(b).to_string();
            assert!(!s.is_empty(), "{b}");
        }
        assert_eq!(DeviceType::TV.to_string(), "TV");
        assert_eq!(DeviceType::RECORDING.to_string(), "Recording Device");
        assert_eq!(DeviceType::PLAYBACK.to_string(), "Playback Device");
    }

    #[test]
    fn power_status_display_covers_all_variants() {
        for b in [0x00u8, 0x01, 0x02, 0x03, 0x7F, 0xFF] {
            assert!(!PowerStatus(b).to_string().is_empty(), "{b:#x}");
        }
        assert_eq!(PowerStatus::ON.to_string(), "On");
        assert_eq!(PowerStatus::STANDBY.to_string(), "Standby");
    }

    #[test]
    fn cec_version_display_covers_known() {
        for b in [0x00u8, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06] {
            assert!(!CECVersion(b).to_string().is_empty(), "{b}");
        }
    }

    #[test]
    fn expected_device_type_maps_all_roles() {
        assert_eq!(expected_device_type(0), DeviceType::TV);
        assert_eq!(expected_device_type(1), DeviceType::RECORDING);
        assert_eq!(expected_device_type(3), DeviceType::TUNER);
        assert_eq!(expected_device_type(4), DeviceType::PLAYBACK);
        assert_eq!(expected_device_type(5), DeviceType::AUDIO_SYSTEM);
        assert_eq!(expected_device_type(12), DeviceType::RESERVED);
        assert_eq!(expected_device_type(255), DeviceType::RESERVED);
    }
}
