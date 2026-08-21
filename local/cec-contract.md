# Contract: Rust libcec FFI layer (`src/cec/`)

You are porting the cgo bindings in `/home/luke/github/capi/cec/*.go` to Rust. Read those files first — they are the semantic reference. Target layout:

```
src/cec/shim.c    # C helpers compiled by build.rs (cc crate)
src/cec/ffi.rs    # extern "C" declarations of shim + cecc.h flat API
src/cec/types.rs  # safe enums/structs, Display impls, vendor table
src/cec/mod.rs    # pub use + Connection (safe API)
```

## Non-negotiable design decisions (fixes for defects found in review)

1. **No struct layouts in Rust.** Replicate the Go pattern: all access to
   `libcec_configuration`, `cec_command`, `cec_logical_addresses`,
   `cec_adapter`, `cec_osd_name`, `cec_menu_language` goes through small C
   helper functions in `shim.c`. Rust declares only opaque handles and flat
   fn signatures. This keeps us immune to libcec6 vs libcec7 layout drift.
2. **Session registry, never dangling callback params.** `Connection::open`
   allocates a `u64` id; global `static SESSIONS: Mutex<HashMap<u64, Arc<SessionInner>>>`.
   `callbackParam` carries the id (`uintptr_t`). Thunks look up the id; unknown id -> return immediately.
3. **No send-on-closed-channel crash.** Events go out on
   `tokio::sync::broadcast::Sender<CecEvent>` (capacity 512). `broadcast::send`
   fails gracefully with no receivers — there is no channel close, so the Go
   teardown panic class cannot exist.
4. **Drain in-flight callbacks before destroy.** `SessionInner` has
   `closing: AtomicBool` and `inflight: AtomicUsize`. Every bridge entry point:
   if `closing` -> return; else `inflight += 1`, re-check `closing` (if flipped,
   decrement and return), copy ALL data out of C structs immediately, dispatch,
   `inflight -= 1`. `close()` order: `closed=true` (CAS) -> api mutex ->
   `libcec_close(handle)` -> remove from SESSIONS -> spin-wait (park/unpark or
   1ms sleep loop, 5s timeout) until `inflight == 0` -> `libcec_destroy(handle)`
   -> drop sender. Document this ordering in mod.rs.
5. **Transmit bounds check** (Go bug): reject `parameters.len() > 64`
   (`CEC_MAX_DATA_PACKET_SIZE`) AND `> 255` size truncation with
   `CecError::InvalidParams`. Also validate initiator/destination are valid
   logical addresses (0..=15) at this layer.
6. **Serialization**: one `Mutex<()>`-style api lock per connection exactly like
   Go's `apiMu` (guard = closed-check, lock, closed-recheck). Bridge callbacks
   must NEVER take the api lock (deadlock-freedom invariant — preserve it).
7. **Transmit timeout**: set `transmit_timeout = 1000` explicitly via a shim
   setter (Go relied on zero-value defaulting — latent bug).

## Shim functions to implement in shim.c

Port these verbatim in behavior from `/home/luke/github/capi/cec/bridges_c.c`:
`cec_set_passive_defaults`, `cec_apply_address_list`, `cec_command_param_byte`,
`cec_command_param_size`, plus new:
- `void* capi_initialise(const char* device_name, int device_type, int monitor_only, int activate_source, const uint8_t* wake, int wake_n, const uint8_t* poweroff, int poweroff_n, uintptr_t cb_param)` — zeroes config, passive defaults, applies overrides, installs process-wide callback table + cb_param, calls `libcec_initialise`. Returns NULL on failure.
- `int capi_install_callbacks_on_set(void* handle, const <same config args>, uintptr_t cb_param)` — used by `set_configuration`: rebuild config, install callbacks, call `libcec_set_configuration`.
- Callback thunks calling function pointers from a global `capi_bridges` struct (log/key/command/config/alert/menu/source), filled once from Rust via `capi_set_bridges(...)` at startup. Each thunk passes `(uintptr_t)cb_param` plus plain-C values only (copy strings/bytes inside the thunk where trivial, e.g. log message as `const char*`, command params via `cec_command_param_byte` loop in Rust like Go does).
- `uint32_t capi_client_version(void)` returns `LIBCEC_VERSION_CURRENT`.

## cecc.h functions to declare in ffi.rs

Exactly the ones the Go code uses (see libcec.go): `libcec_find_adapters`,
`libcec_open`, `libcec_close`, `libcec_power_on_devices`,
`libcec_standby_devices`, `libcec_set_active_source`,
`libcec_set_inactive_view`, `libcec_volume_up/down`,
`libcec_audio_toggle_mute/mute/unmute`, `libcec_get_device_power_status`,
`libcec_get_active_source`, `libcec_is_active_source`,
`libcec_get_device_vendor_id`, `libcec_get_device_physical_address`,
`libcec_get_device_osd_name`, `libcec_get_device_menu_language`,
`libcec_get_device_cec_version`, `libcec_get_active_devices`,
`libcec_is_active_device`, `libcec_transmit`, `libcec_send_keypress`,
`libcec_send_key_release`, `libcec_set_osd_string`, `libcec_switch_monitoring`,
`libcec_get_lib_info`, `libcec_set_configuration`,
`libcec_get_current_configuration`, `libcec_audio_get_status`,
`libcec_poll_device`, `libcec_set_hdmi_port`, `libcec_rescan_devices`,
`libcec_get_logical_addresses`, `libcec_initialise`, `libcec_destroy`.
Link via pkg-config `libcec` (build.rs: `pkg-config` not required — emit
`cargo:rustc-link-lib=dylib=cec` + `p8_platform` and rely on default search
paths; add `cargo:rustc-link-search` from `PKG_CONFIG_PATH` if set).

## Safe types (types.rs)

Port from `/home/luke/github/capi/cec/types.go` and `helpers.go`:
`LogicalAddress(u8)` with `is_valid()` (<=14) + `Display` names ("TV",
"RecordingDevice", ... match Go strings EXACTLY — they appear in JSON),
`Opcode(u8)` with the full name table from Go (opcodeName/opcodeNames),
`Keycode(u8)` with the full 60+ name map from Go keyNameMap (canonical
lowercase underscore names), `DeviceType`, `PowerStatus` (+ string mapping
used by powerStatusFromByte in capi/cec_events.go: "on"/"standby"/...),
`CECVersion`, `DisplayControl`, `Command { initiator, destination, opcode,
opcode_set, parameters: Vec<u8>, ack, eom }`, `Adapter { path, comm }`,
`DeviceInfo` aggregation equivalent to Go `GetDeviceInfo` (osd_name, vendor_id
hex string "0x%06x", vendor_name from the known-vendor table in helpers.go,
cec_version, power_status, physical_address formatted "%x.%x.%x", hdmi_port
derived like Go), `MenuState`.

## CecEvent enum (delivered on broadcast channel)

```rust
pub enum CecEvent {
    Log { level: LogLevel, time: i64, message: String },
    KeyPress { key: u8, duration: u32 },
    Command(Command),          // copied out fully before dispatch
    ConfigurationChanged(Configuration),
    Alert { alert: i32, param_type: i32, param_value: i64 },
    MenuState { state: i32 },  // menu handler ALSO still called synchronously
    SourceActivated { address: u8, activated: bool },
}
```

## Connection public API (mod.rs) — mirror Go method-for-method

open(config: &Configuration) -> Result<Connection>; find_adapters;
open_adapter(path); close() (idempotent); is_closed; is_monitor_only;
server_version() -> u32 (from get_current_configuration);
power_on/standby(addr 0..=14 validated); set_active_source(DeviceType);
set_inactive_view; volume_up/down(send_release); audio_toggle_mute/mute/unmute;
get_device_power_status (CEC_POWER_STATUS_UNKNOWN == 0x99 -> error, note Go
sentinel mismatch: keep 0x99 as the wire value, expose PowerStatus::Unknown);
get_active_source (invalid -> ErrNoActiveSource); is_active_source;
get_device_vendor_id (CEC_VENDOR_UNKNOWN -> error); get_device_physical_address
(CEC_INVALID_PHYSICAL_ADDRESS 0xFFFF -> error); get_device_osd_name ([14]char);
get_device_menu_language ([4]char); get_device_cec_version; get_active_devices
(bitmask -> Vec); is_active_device; transmit(&Command) WITH bounds checks;
send_keypress/send_key_release (monitor-only refusal); set_osd_string;
switch_monitoring; get_lib_info; set_configuration(&Configuration);
get_current_configuration; get_audio_status -> (volume, muted, raw);
poll_device; set_hdmi_port(port 1..=15); rescan_devices(settle: Duration)
(release lock during settle like Go); get_logical_addresses (empty-mask +
primary fallback like Go); ping_tv() = get_device_power_status(TV).
All blocking fns are sync (callers wrap in spawn_blocking). Mark the module
`unsafe` boundary clearly: every unsafe block cites the invariant it relies on.

## Configuration struct

```rust
pub struct Configuration {
    pub device_name: String,       // <=13 chars, truncate
    pub device_type: DeviceType,
    pub physical_address: u16,     // 0xFFFF = auto
    pub base_device: LogicalAddress,
    pub hdmi_port: u8,
    pub monitor_only: bool,
    pub activate_source: bool,
    pub wake_devices: Vec<u8>,
    pub power_off_devices: Vec<u8>,
}
```

## Tests (cargo test, no hardware)

Pure parts only: keycode name round-trip, opcode naming, logical address
display strings, physical address formatting/parsing, vendor name lookup,
Configuration device-name truncation, Transmit parameter validation (use a
testable validation fn that does not need a live session).

## Acceptance

- `cargo build` succeeds linking system libcec (installed: libcec 7.1.1, headers at /usr/include/libcec).
- `cargo test -p . cec::` green.
- `cargo clippy -- -D warnings` clean in your files.
- Do NOT touch anything outside src/cec/. Do NOT modify build.rs unless adding the pkg-config link lines (coordinate: append only).
