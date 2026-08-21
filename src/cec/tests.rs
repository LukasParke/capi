//! Pure-logic tests for the cec FFI layer (no hardware, no live session).
//! Covers the contract's test list: keycode name round-trip, opcode
//! naming, logical address display strings, physical address
//! formatting/parsing, vendor name lookup, Configuration device-name
//! truncation, and Transmit parameter validation.

use super::ffi::CEC_MAX_DATA_PACKET_SIZE;
use super::types::*;
use super::{linked_libcec_major, validate_transmit};

#[test]
fn logical_address_display_strings_match_go() {
    // Exact Go LogicalAddress.String() wire strings.
    assert_eq!(LogicalAddress::TV.to_string(), "TV");
    assert_eq!(
        LogicalAddress::RECORDING_DEVICE_1.to_string(),
        "Recording Device 1"
    );
    assert_eq!(LogicalAddress::TUNER_1.to_string(), "Tuner 1");
    assert_eq!(
        LogicalAddress::PLAYBACK_DEVICE_1.to_string(),
        "Playback Device 1"
    );
    assert_eq!(LogicalAddress::AUDIO_SYSTEM.to_string(), "Audio System");
    assert_eq!(
        LogicalAddress::PLAYBACK_DEVICE_3.to_string(),
        "Playback Device 3"
    );
    assert_eq!(LogicalAddress::FREE_USE.to_string(), "Free Use");
    assert_eq!(LogicalAddress::BROADCAST.to_string(), "Broadcast");
    assert_eq!(LogicalAddress::UNKNOWN.to_string(), "Unknown");
    assert_eq!(logical_address_name(0x05), "Audio System");

    // Validity: 0..=14 valid; broadcast and unknown are not.
    assert!(LogicalAddress(0).is_valid());
    assert!(LogicalAddress(14).is_valid());
    assert!(!LogicalAddress(15).is_valid());
    assert!(!LogicalAddress(0xFF).is_valid());
    assert!(LogicalAddress(15).is_broadcast());
    assert!(LogicalAddress(0xFF).is_unknown());
}

#[test]
fn keycode_names_round_trip() {
    for (name, code) in KEY_NAMES {
        let resolved = keycode_from_name(name);
        assert_eq!(
            resolved,
            Some(*code),
            "name {name:?} must resolve to its code"
        );
        if let Some(canonical) = code.canonical_name() {
            assert_eq!(
                keycode_from_name(canonical),
                Some(*code),
                "canonical name {canonical:?} must round-trip to {code:?}"
            );
        } else {
            panic!("keycode {code:?} from table has no canonical name");
        }
    }
    // Aliases collapse onto shared codes but resolve cleanly.
    assert_eq!(keycode_from_name("home"), Some(Keycode::ROOT_MENU));
    assert_eq!(keycode_from_name("back"), Some(Keycode::EXIT));
    assert_eq!(keycode_from_name("menu"), Some(Keycode::SETUP_MENU));
    // Unknown names are None.
    assert_eq!(keycode_from_name("does_not_exist"), None);
    // Spot-check raw values against the CEC spec.
    assert_eq!(Keycode::SELECT.0, 0x00);
    assert_eq!(Keycode::K9.0, 0x29);
    assert_eq!(Keycode::POWER.0, 0x40);
    assert_eq!(Keycode::F4_YELLOW.0, 0x74);
}

#[test]
fn opcode_naming_matches_go_table_and_hex_fallback() {
    assert_eq!(
        opcode_name(Opcode::REPORT_POWER_STATUS),
        "REPORT_POWER_STATUS"
    );
    assert_eq!(opcode_name(Opcode::FEATURE_ABORT), "FEATURE_ABORT");
    assert_eq!(
        opcode_name(Opcode::USER_CONTROL_RELEASED),
        "USER_CONTROL_RELEASED"
    );
    assert_eq!(opcode_name(Opcode::STANDBY), "STANDBY");
    // Not in the Go table -> uppercase hex fallback.
    assert_eq!(opcode_name(Opcode::GIVE_PHYSICAL_ADDRESS), "0x83");
    assert_eq!(opcode_name(Opcode(0xAB)), "0xAB");
    // Opcode::name mirrors the free fn.
    assert_eq!(Opcode::IMAGE_VIEW_ON.name(), "IMAGE_VIEW_ON");
}

#[test]
fn physical_address_format_parse_round_trip() {
    assert_eq!(physical_address_to_string(0x2100), "2.1.0.0");
    assert_eq!(physical_address_to_string(0x0000), "0.0.0.0");
    assert_eq!(physical_address_to_string(0xFFFF), "15.15.15.15");
    for packed in [0u16, 0x1000, 0x2100, 0x3210, 0xFFFF] {
        let dotted = physical_address_to_string(packed);
        assert_eq!(parse_physical_address(&dotted).unwrap(), packed);
    }
    assert!(parse_physical_address("2.1").is_err());
    assert!(parse_physical_address("2.1.0.0.0").is_err());
    assert!(parse_physical_address("16.0.0.0").is_err());
    assert!(parse_physical_address("a.b.c.d").is_err());
    assert_eq!(parse_physical_address("1.0.0.0").unwrap(), 0x1000);
}

#[test]
fn vendor_lookup_known_and_unknown() {
    assert_eq!(get_vendor_name(0x0000F0), "Samsung");
    assert_eq!(get_vendor_name(0x08001F), "Sony");
    assert_eq!(get_vendor_name(0x001582), "Pulse Eight");
    assert!(is_known_vendor(0x008045));
    assert!(!is_known_vendor(0x123456));
    // Unknown IDs format like Go GetVendorName.
    assert_eq!(get_vendor_name(0xABCDEF), "Unknown (0xABCDEF)");
}

#[test]
fn device_name_truncated_to_thirteen_bytes() {
    assert_eq!(sanitize_device_name("capi"), "capi");
    assert_eq!(sanitize_device_name(""), "");
    // Exactly at the limit stays intact.
    assert_eq!(sanitize_device_name("abcdefghijklm"), "abcdefghijklm");
    // Longer names truncate to 13 bytes.
    assert_eq!(sanitize_device_name("abcdefghijklmnop"), "abcdefghijklm");
    // Interior NUL cannot survive into a CString.
    assert!(!sanitize_device_name("ab\0cd").contains('\0'));
    // Multi-byte characters truncate on a byte boundary without panicking.
    let truncated = sanitize_device_name("éééééééé");
    assert!(truncated.len() <= 13);
}

#[test]
fn transmit_validation_bounds_checks() {
    let base = Command {
        initiator: LogicalAddress(0),
        destination: LogicalAddress(4),
        opcode: Opcode(0x64),
        opcode_set: true,
        parameters: vec![1, 2, 3],
        ack: false,
        eom: true,
    };
    assert!(validate_transmit(&base).is_ok());

    // > 64 parameter bytes rejected (CEC_MAX_DATA_PACKET_SIZE).
    let too_long = Command {
        parameters: vec![0u8; CEC_MAX_DATA_PACKET_SIZE + 1],
        ..base.clone()
    };
    assert!(matches!(
        validate_transmit(&too_long),
        Err(CecError::InvalidParams(_))
    ));
    // Exactly 64 is fine.
    let full = Command {
        parameters: vec![0u8; CEC_MAX_DATA_PACKET_SIZE],
        ..base.clone()
    };
    assert!(validate_transmit(&full).is_ok());

    // Unknown address (0xFF) rejected on either side.
    let bad_initiator = Command {
        initiator: LogicalAddress(0xFF),
        ..base.clone()
    };
    assert!(matches!(
        validate_transmit(&bad_initiator),
        Err(CecError::InvalidParams(_))
    ));
    let bad_destination = Command {
        destination: LogicalAddress(0xFF),
        ..base
    };
    assert!(matches!(
        validate_transmit(&bad_destination),
        Err(CecError::InvalidParams(_))
    ));
}

#[test]
fn device_info_helpers_match_go_derivation() {
    assert_eq!(DeviceInfo::derive_hdmi_port(0x2100), 2);
    assert_eq!(DeviceInfo::derive_hdmi_port(0x0000), 0);
    assert_eq!(DeviceInfo::derive_hdmi_port(0xFFFF), 0);

    // power_status_str matches Go powerStatusFromByte.
    assert_eq!(power_status_str(0x00), "on");
    assert_eq!(power_status_str(0x01), "standby");
    assert_eq!(power_status_str(0x02), "transitioning_to_on");
    assert_eq!(power_status_str(0x03), "transitioning_to_standby");
    assert_eq!(power_status_str(0x99), "unknown");
    assert_eq!(power_status_str(0xFF), "unknown");

    assert_eq!(device_type_for_address(4), "PlaybackDevice1");
    assert_eq!(device_type_for_address(0), "TV");
    assert_eq!(device_type_for_address(5), "AudioSystem");
    assert_eq!(device_type_for_address(0xFF), "Unknown");
    assert_eq!(expected_device_type(1), DeviceType::RECORDING);

    // DeviceInfo::to_map produces the documented JSON keys.
    let info = DeviceInfo {
        logical_address: LogicalAddress(0),
        address_name: "TV".into(),
        physical_address: 0x2100,
        osd_name: "TV".into(),
        menu_language: "eng".into(),
        vendor_id: 0xF0,
        vendor_name: "Samsung".into(),
        vendor_known: true,
        cec_version: CECVersion::V1_4,
        power_status: PowerStatus::ON,
        hdmi_port: 2,
        is_active: true,
        is_active_source: false,
        errors: DeviceInfoErrors::default(),
    };
    let m = info.to_map();
    assert_eq!(m["logical_address"], serde_json::json!(0));
    assert_eq!(m["address_name"], serde_json::json!("TV"));
    assert_eq!(m["physical_address"], serde_json::json!("2.1.0.0"));
    assert_eq!(m["vendor_id"], serde_json::json!("0x0000f0"));
    assert_eq!(m["vendor_name"], serde_json::json!("Samsung"));
    assert_eq!(m["vendor_known"], serde_json::json!(true));
    assert_eq!(m["cec_version"], serde_json::json!("1.4"));
    assert_eq!(m["power_status"], serde_json::json!("On"));
    assert_eq!(m["hdmi_port"], serde_json::json!(2));
}

#[test]
fn linked_libcec_major_reports_seven() {
    // System libcec is 7.1.1; the runtime-linked major must be >= 7 and
    // match the header's LIBCEC_VERSION_MAJOR.
    assert_eq!(linked_libcec_major(), 7);
}

#[test]
fn keycode_and_opcode_tables_sorted_and_resolvable() {
    let keys = super::keycode_names();
    assert!(!keys.is_empty());
    assert!(keys
        .windows(2)
        .all(|w| (w[0].1, &w[0].0) <= (w[1].1, &w[1].0)));
    for (name, code) in &keys {
        assert_eq!(
            keycode_from_name(name).map(|k| k.0),
            Some(*code),
            "every listed name must resolve via keycode_from_name"
        );
    }

    let ops = super::opcode_table();
    assert!(!ops.is_empty());
    assert!(ops.windows(2).all(|w| w[0].1 <= w[1].1));
    for (name, op) in &ops {
        assert_eq!(&opcode_name(Opcode(*op)), name);
    }
}
