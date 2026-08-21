//! Raw FFI declarations: the flat cecc.h API plus our C shim (`shim.c`).
//!
//! Design rule (see local://cec-contract.md §1): Rust never replicates a
//! libcec struct layout. Every function here takes/returns plain integers,
//! pointers to opaque data, or pointers to byte buffers. Struct-shaped
//! access goes through shim helpers in `shim.c`.
//!
//! # Safety contract for callers
//! All `handle: *mut c_void` arguments must be the pointer returned by
//! [`capi_initialise`] (via [`crate::cec::Connection::open`]) and must not
//! have been destroyed yet. The safe wrapper in `mod.rs` enforces this with
//! its close/drain ordering.

use std::os::raw::{c_char, c_int, c_void};

/// Opaque libcec session handle (`libcec_connection_t` in C mode).
pub type LibcecHandle = *mut c_void;

// Constants from cectypes.h (values pinned by the C headers; verified
// against libcec 7.1.1).
pub const LIBCEC_OSD_NAME_SIZE: usize = 15;
/// Maximum bytes in a single CEC frame's parameter block.
pub const CEC_MAX_DATA_PACKET_SIZE: usize = 64;
pub const CEC_INVALID_PHYSICAL_ADDRESS: u16 = 0xFFFF;
pub const CEC_VENDOR_UNKNOWN: u32 = 0;
pub const CEC_VERSION_UNKNOWN: u32 = 0x00;
/// libcec's wire sentinel for "power status unknown" — note this differs
/// from Go's `PowerStatusUnknown` (0xFF); kept distinct on purpose.
pub const CEC_POWER_STATUS_UNKNOWN: i32 = 0x99;
/// "not a valid logical address" sentinel (enum value -1).
pub const CECDEVICE_UNKNOWN: i32 = -1;
pub const CEC_DEFAULT_TRANSMIT_TIMEOUT_MS: i32 = 1000;
/// Timeout used by Go's OpenAdapter.
pub const ADAPTER_OPEN_TIMEOUT_MS: u32 = 5000;
/// Buffer size Go used for libcec_find_adapters.
pub const MAX_ADAPTERS: usize = 10;
/// sizeof(cec_adapter.path) / sizeof(cec_adapter.comm).
pub const CEC_ADAPTER_STR_SIZE: usize = 1024;

// ---------------------------------------------------------------------------
// cecc.h flat API (exactly the functions the Go bindings use).
// ---------------------------------------------------------------------------
extern "C" {
    pub fn libcec_initialise(configuration: *mut c_void) -> LibcecHandle;
    pub fn libcec_destroy(connection: LibcecHandle);
    /// Wrapped by [`capi_find_adapters`] — declared for completeness; do not
    /// call directly from Rust (takes a `cec_adapter` array).
    pub fn libcec_find_adapters(
        connection: LibcecHandle,
        device_list: *mut c_void,
        buf_size: u8,
        device_path: *const c_char,
    ) -> i8;
    pub fn libcec_open(connection: LibcecHandle, port: *const c_char, timeout_ms: u32) -> c_int;
    pub fn libcec_close(connection: LibcecHandle);

    pub fn libcec_power_on_devices(connection: LibcecHandle, address: c_int) -> c_int;
    pub fn libcec_standby_devices(connection: LibcecHandle, address: c_int) -> c_int;
    pub fn libcec_set_active_source(connection: LibcecHandle, device_type: c_int) -> c_int;
    pub fn libcec_set_inactive_view(connection: LibcecHandle) -> c_int;
    pub fn libcec_volume_up(connection: LibcecHandle, send_release: c_int) -> c_int;
    pub fn libcec_volume_down(connection: LibcecHandle, send_release: c_int) -> c_int;
    pub fn libcec_audio_toggle_mute(connection: LibcecHandle) -> u8;
    pub fn libcec_audio_mute(connection: LibcecHandle) -> u8;
    pub fn libcec_audio_unmute(connection: LibcecHandle) -> u8;
    pub fn libcec_audio_get_status(connection: LibcecHandle) -> u8;

    pub fn libcec_get_device_power_status(connection: LibcecHandle, address: c_int) -> i32;
    pub fn libcec_get_active_source(connection: LibcecHandle) -> i32;
    pub fn libcec_is_active_source(connection: LibcecHandle, address: c_int) -> c_int;
    pub fn libcec_get_device_vendor_id(connection: LibcecHandle, address: c_int) -> u32;
    pub fn libcec_get_device_physical_address(connection: LibcecHandle, address: c_int) -> u16;
    // Out params are fixed-size char arrays on the C side ([14] / [4]); we
    // route through capi_get_device_osd_name / capi_get_device_menu_language
    // which copy into caller-owned byte buffers.
    pub fn libcec_get_device_cec_version(connection: LibcecHandle, address: c_int) -> i32;
    pub fn libcec_is_active_device(connection: LibcecHandle, address: c_int) -> c_int;
    pub fn libcec_transmit(connection: LibcecHandle, command: *const c_void) -> c_int;
    pub fn libcec_send_keypress(
        connection: LibcecHandle,
        destination: c_int,
        key: c_int,
        wait: c_int,
    ) -> c_int;
    pub fn libcec_send_key_release(
        connection: LibcecHandle,
        destination: c_int,
        wait: c_int,
    ) -> c_int;
    pub fn libcec_set_osd_string(
        connection: LibcecHandle,
        address: c_int,
        duration: c_int,
        message: *const c_char,
    ) -> c_int;
    pub fn libcec_switch_monitoring(connection: LibcecHandle, enable: c_int) -> c_int;
    pub fn libcec_get_lib_info(connection: LibcecHandle) -> *const c_char;
    pub fn libcec_set_configuration(
        connection: LibcecHandle,
        configuration: *const c_void,
    ) -> c_int;
    /// Wrapped by [`capi_get_current_configuration`] (out struct).
    pub fn libcec_get_current_configuration(
        connection: LibcecHandle,
        configuration: *mut c_void,
    ) -> c_int;
    pub fn libcec_poll_device(connection: LibcecHandle, address: c_int) -> c_int;
    pub fn libcec_set_hdmi_port(connection: LibcecHandle, base_device: c_int, port: u8) -> c_int;
    pub fn libcec_rescan_devices(connection: LibcecHandle);
    pub fn libcec_system_audio_mode(connection: LibcecHandle, enable: c_int) -> c_int;
}

// ---------------------------------------------------------------------------
// Our shim (src/cec/shim.c), compiled by build.rs.
// ---------------------------------------------------------------------------
#[repr(C)]
pub struct capi_bridges {
    pub log: Option<unsafe extern "C" fn(id: usize, level: i32, time: i64, message: *const c_char)>,
    pub key: Option<unsafe extern "C" fn(id: usize, keycode: i32, duration: u32)>,
    pub command: Option<unsafe extern "C" fn(id: usize, cmd: *const c_void)>,
    pub config_changed: Option<unsafe extern "C" fn(id: usize, cfg: *const c_void)>,
    pub alert:
        Option<unsafe extern "C" fn(id: usize, alert: i32, param_type: i32, param_value: i64)>,
    pub menu: Option<unsafe extern "C" fn(id: usize, state: i32) -> c_int>,
    pub source: Option<unsafe extern "C" fn(id: usize, address: i32, activated: i32)>,
}

extern "C" {
    pub fn capi_set_bridges(bridges: *const capi_bridges);
    pub fn capi_client_version() -> u32;

    pub fn capi_apply_address_list(dest: *mut c_void, addrs: *const u8, n: c_int);
    pub fn capi_build_config(
        device_name: *const c_char,
        device_type: c_int,
        physical_address: u16,
        base_device: c_int,
        hdmi_port: u8,
        client_version: u32,
        monitor_only: c_int,
        activate_source: c_int,
        wake: *const u8,
        wake_n: c_int,
        poweroff: *const u8,
        poweroff_n: c_int,
    ) -> *mut c_void;
    pub fn capi_free_config(cfg: *mut c_void);

    pub fn capi_initialise(
        device_name: *const c_char,
        device_type: c_int,
        physical_address: u16,
        base_device: c_int,
        hdmi_port: u8,
        monitor_only: c_int,
        activate_source: c_int,
        wake: *const u8,
        wake_n: c_int,
        poweroff: *const u8,
        poweroff_n: c_int,
        cb_param: usize,
    ) -> LibcecHandle;
    pub fn capi_install_callbacks_on_set(
        handle: LibcecHandle,
        device_name: *const c_char,
        device_type: c_int,
        physical_address: u16,
        base_device: c_int,
        hdmi_port: u8,
        monitor_only: c_int,
        activate_source: c_int,
        wake: *const u8,
        wake_n: c_int,
        poweroff: *const u8,
        poweroff_n: c_int,
        cb_param: usize,
    ) -> c_int;

    pub fn capi_command_param_byte(cmd: *const c_void, index: c_int) -> u8;
    pub fn capi_command_param_size(cmd: *const c_void) -> u8;
    pub fn capi_command_init(
        cmd: *mut c_void,
        initiator: c_int,
        destination: c_int,
        opcode: c_int,
        opcode_set: c_int,
    );
    pub fn capi_command_push_params(cmd: *mut c_void, data: *const u8, n: c_int);
    pub fn capi_command_set_transmit_timeout(cmd: *mut c_void, ms: i32);

    pub fn capi_find_adapters(
        handle: LibcecHandle,
        bufsize: u8,
        paths: *mut c_char,
        comms: *mut c_char,
        slot: usize,
    ) -> i8;
    pub fn capi_get_active_devices_mask(handle: LibcecHandle) -> u16;
    pub fn capi_get_logical_addresses_mask(handle: LibcecHandle, primary_out: *mut c_int) -> u16;
    pub fn capi_get_device_osd_name(
        handle: LibcecHandle,
        address: c_int,
        out: *mut c_char,
        out_size: usize,
    ) -> c_int;
    pub fn capi_get_device_menu_language(
        handle: LibcecHandle,
        address: c_int,
        out: *mut c_char,
        out_size: usize,
    ) -> c_int;

    pub fn capi_config_device_name(cfg: *const c_void) -> *const c_char;
    pub fn capi_config_device_type(cfg: *const c_void) -> c_int;
    pub fn capi_config_physical_address(cfg: *const c_void) -> u16;
    pub fn capi_config_base_device(cfg: *const c_void) -> c_int;
    pub fn capi_config_hdmi_port(cfg: *const c_void) -> u8;
    pub fn capi_config_client_version(cfg: *const c_void) -> u32;
    pub fn capi_config_server_version(cfg: *const c_void) -> u32;
}

/// Our own snapshot struct for `libcec_get_current_configuration` — a plain
/// C struct owned by the shim (see `capi_config_out` in shim.c), NOT a
/// libcec layout. Field types must match the shim exactly.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct capi_config_out {
    pub device_name: [c_char; LIBCEC_OSD_NAME_SIZE],
    pub device_type: c_int,
    pub physical_address: u16,
    pub base_device: c_int,
    pub hdmi_port: u8,
    pub client_version: u32,
    pub server_version: u32,
}

extern "C" {
    pub fn capi_get_current_configuration(handle: LibcecHandle, out: *mut capi_config_out)
        -> c_int;
}

extern "C" {
    pub fn capi_command_initiator(cmd: *const c_void) -> u8;
    pub fn capi_command_destination(cmd: *const c_void) -> u8;
    pub fn capi_command_ack(cmd: *const c_void) -> c_int;
    pub fn capi_command_eom(cmd: *const c_void) -> c_int;
    pub fn capi_command_opcode(cmd: *const c_void) -> u8;
    pub fn capi_command_opcode_set(cmd: *const c_void) -> u8;
}
