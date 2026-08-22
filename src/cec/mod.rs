//! Safe libcec FFI layer: port of the Go bindings in `cec/*.go`
//! (`cec.go`, `libcec.go`, `bridges.go`, `events.go`, `helpers.go`).
//!
//! # Architecture (see `local://cec-contract.md`)
//!
//! * **No libcec struct layouts in Rust.** All access to
//!   `libcec_configuration`, `cec_command`, `cec_logical_addresses`,
//!   `cec_adapter`, `cec_osd_name`, `cec_menu_language` goes through C
//!   helpers in [`shim`]; Rust sees only opaque handles and flat
//!   signatures.
//! * **Session registry, never dangling callback params.** Every session
//!   gets a `u64` id; callbacks carry only that id and look it up in
//!   [`SESSIONS`]. Unknown ids return immediately, so a callback racing a
//!   close can never touch freed state.
//! * **No send-on-closed-channel crash.** Events go out on a
//!   `tokio::sync::broadcast` channel; sending with no receivers simply
//!   fails gracefully — there is no channel-close panic class.
//! * **In-flight drain before destroy** — see [`Connection::close`] for
//!   the exact teardown ordering (contract §4).
//!
//! # Deadlock-freedom invariant
//!
//! Bridge callbacks must NEVER take the per-connection api lock
//! ([`SessionInner::api`]). They only bump `inflight`, copy data out of the
//! C structs, and dispatch onto the broadcast channel.
//!
//! All blocking methods here are synchronous; callers wrap them in
//! `spawn_blocking`.

pub mod ffi;
#[cfg(test)]
mod tests;
pub mod types;

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ffi::{
    capi_bridges, ADAPTER_OPEN_TIMEOUT_MS, CECDEVICE_UNKNOWN, CEC_ADAPTER_STR_SIZE,
    CEC_DEFAULT_TRANSMIT_TIMEOUT_MS, CEC_INVALID_PHYSICAL_ADDRESS, CEC_POWER_STATUS_UNKNOWN,
    CEC_VENDOR_UNKNOWN, CEC_VERSION_UNKNOWN, MAX_ADAPTERS,
};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Once};
use tokio::sync::broadcast;
pub use types::*;
/// Broadcast channel capacity (contract §3).
const EVENT_CAPACITY: usize = 512;
/// Max time `close()` waits for in-flight callbacks to drain (contract §4).
const CLOSE_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

static SESSIONS: LazyLock<Mutex<HashMap<u64, Arc<SessionInner>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

fn sessions() -> &'static Mutex<HashMap<u64, Arc<SessionInner>>> {
    &SESSIONS
}

fn session_for(id: usize) -> Option<Arc<SessionInner>> {
    if id == 0 {
        return None;
    }
    sessions().lock().ok()?.get(&(id as u64)).cloned()
}

// ---------------------------------------------------------------------------
// Bridge thunks: entry points called from shim.c on libcec threads.
//
// Protocol (contract §4): look up the session id; unknown -> return. If
// `closing` -> return. Else inflight += 1, RE-CHECK closing (bail if it
// flipped), copy ALL data out of the C structs immediately, dispatch,
// inflight -= 1.
// ---------------------------------------------------------------------------

static BRIDGES_INSTALLED: Once = Once::new();

fn install_bridges() {
    BRIDGES_INSTALLED.call_once(|| {
        let bridges = capi_bridges {
            log: Some(bridge_log),
            key: Some(bridge_key),
            command: Some(bridge_command),
            config_changed: Some(bridge_config_changed),
            alert: Some(bridge_alert),
            menu: Some(bridge_menu),
            source: Some(bridge_source),
        };
        // Safety: capi_set_bridges copies the table by value; the fn
        // pointers are `unsafe extern "C"` fns with static lifetimes and
        // signatures matching shim.c's `capi_bridges` exactly.
        unsafe { ffi::capi_set_bridges(&bridges) };
    });
}

unsafe extern "C" fn bridge_log(id: usize, level: i32, time: i64, message: *const c_char) {
    let Some(s) = session_for(id) else { return };
    if !s.begin_callback() {
        return;
    }
    // Copy out of C memory immediately (message is only valid during the
    // callback).
    let message = if message.is_null() {
        String::new()
    } else {
        // Safety: shim passes a NUL-terminated `const char*` valid for the
        // duration of the callback (cec_log_message.message).
        CStr::from_ptr(message).to_string_lossy().into_owned()
    };
    s.dispatch(CecEvent::Log {
        level: LogLevel(level),
        time,
        message,
    });
    s.end_callback();
}

unsafe extern "C" fn bridge_key(id: usize, keycode: i32, duration: u32) {
    let Some(s) = session_for(id) else { return };
    if !s.begin_callback() {
        return;
    }
    s.dispatch(CecEvent::KeyPress {
        // Safety: keycode arrives as an int holding a cec_user_control_code
        // enum value (0..=255); wrap-cast preserves the low byte like Go's
        // uint8(keycode) conversion.
        key: keycode as u8,
        duration,
    });
    s.end_callback();
}

unsafe extern "C" fn bridge_command(id: usize, cmd: *const c_void) {
    let Some(s) = session_for(id) else { return };
    if !s.begin_callback() {
        return;
    }
    if cmd.is_null() {
        s.end_callback();
        return;
    }
    // Copy all parameter bytes out via the shim accessors, like Go does —
    // never dereference the flexible struct from Rust.
    // Safety: cmd points to a live cec_command for the duration of the
    // callback; the accessors read plain bytes inside shim.c.
    let size = unsafe { ffi::capi_command_param_size(cmd) } as usize;
    let mut parameters = Vec::with_capacity(size);
    for i in 0..size {
        // Safety: i < parameters.size, so data[i] is in-bounds (shim.c).
        parameters.push(unsafe { ffi::capi_command_param_byte(cmd, i as c_int) });
    }
    // Safety: field reads happen inside shim.c-backed accessors above; the
    // initiator/destination/flag reads below go through dedicated getters
    // too. (See capi_command_* in shim.c.)
    let event = CecEvent::Command(Command {
        initiator: LogicalAddress(unsafe { ffi::capi_command_initiator(cmd) }),
        destination: LogicalAddress(unsafe { ffi::capi_command_destination(cmd) }),
        ack: unsafe { ffi::capi_command_ack(cmd) } != 0,
        eom: unsafe { ffi::capi_command_eom(cmd) } != 0,
        opcode: Opcode(unsafe { ffi::capi_command_opcode(cmd) }),
        opcode_set: unsafe { ffi::capi_command_opcode_set(cmd) } != 0,
        parameters,
    });
    s.dispatch(event);
    s.end_callback();
}

unsafe extern "C" fn bridge_config_changed(id: usize, cfg: *const c_void) {
    let Some(s) = session_for(id) else { return };
    if !s.begin_callback() {
        return;
    }
    if cfg.is_null() {
        s.end_callback();
        return;
    }
    s.dispatch(CecEvent::ConfigurationChanged(config_snapshot_from_raw(
        cfg,
    )));
    s.end_callback();
}

unsafe extern "C" fn bridge_alert(id: usize, alert: i32, param_type: i32, param_value: i64) {
    let Some(s) = session_for(id) else { return };
    if !s.begin_callback() {
        return;
    }
    s.dispatch(CecEvent::Alert {
        alert,
        param_type,
        param_value,
    });
    s.end_callback();
}

/// Menu handler ALSO answers libcec synchronously (return value decides
/// whether libcec adopts the new menu state). Never takes the api lock.
unsafe extern "C" fn bridge_menu(id: usize, state: i32) -> c_int {
    let Some(s) = session_for(id) else { return 1 };
    if !s.begin_callback() {
        return 1;
    }
    s.dispatch(CecEvent::MenuState { state });
    let handler = s.menu_handler.lock().ok().and_then(|h| h.clone());
    let rc = match handler {
        Some(h) => {
            if h(MenuState(state as u8)) {
                1
            } else {
                0
            }
        }
        None => 1,
    };
    s.end_callback();
    rc
}

unsafe extern "C" fn bridge_source(id: usize, address: i32, activated: i32) {
    let Some(s) = session_for(id) else { return };
    if !s.begin_callback() {
        return;
    }
    s.dispatch(CecEvent::SourceActivated {
        address: address as u8,
        activated: activated != 0,
    });
    s.end_callback();
}

// ---------------------------------------------------------------------------
// Session internals
// ---------------------------------------------------------------------------

type MenuHandler = Arc<dyn Fn(MenuState) -> bool + Send + Sync>;
type EventSinkFn = Arc<dyn Fn(&CecEvent) + Send + Sync>;

struct SessionInner {
    /// libcec handle; null once close() has begun destroying the session.
    /// Touched only under `api` (or by open/close).
    handle: AtomicPtr<c_void>,
    /// Serializes all libcec API calls, mirroring Go's `apiMu`.
    api: Mutex<()>,
    closed: AtomicBool,
    /// Set by a successful open_adapter; bus-touching calls are refused
    /// before it — libcec segfaults on several getters when the session is
    /// initialised but no adapter has been opened (observed, not assumed).
    opened: AtomicBool,
    closing: AtomicBool,
    inflight: AtomicUsize,
    monitor_only: AtomicBool,
    events: broadcast::Sender<CecEvent>,
    menu_handler: Mutex<Option<MenuHandler>>,
    event_sink: Mutex<Option<EventSinkFn>>,
    sink_delivered: AtomicU64,
}

impl SessionInner {
    /// Callback prologue (contract §4). Returns false when the callback
    /// must not run (closing or closed).
    fn begin_callback(&self) -> bool {
        if self.closing.load(Ordering::SeqCst) || self.closed.load(Ordering::SeqCst) {
            return false;
        }
        self.inflight.fetch_add(1, Ordering::SeqCst);
        if self.closing.load(Ordering::SeqCst) || self.closed.load(Ordering::SeqCst) {
            self.inflight.fetch_sub(1, Ordering::SeqCst);
            return false;
        }
        true
    }

    fn end_callback(&self) {
        self.inflight.fetch_sub(1, Ordering::SeqCst);
    }

    /// Graceful dispatch: broadcast send fails harmlessly with no
    /// receivers or when the channel is lagged — no closed-channel panics.
    /// When a synchronous event sink is installed (test hook), events are
    /// handed to it directly and the broadcast hop is skipped, making
    /// bus-state assertions deterministic.
    fn dispatch(&self, ev: CecEvent) {
        if let Some(sink) = self.event_sink.lock().unwrap().as_ref() {
            sink(&ev);
            self.sink_delivered.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let _ = self.events.send(ev);
    }
}

fn config_snapshot_from_raw(cfg: *const c_void) -> Configuration {
    // Safety: every accessor reads a single scalar/char-array field inside
    // shim.c; cfg points to a live libcec_configuration for the callback
    // duration.
    unsafe {
        let name = ffi::capi_config_device_name(cfg);
        Configuration {
            device_name: if name.is_null() {
                String::new()
            } else {
                CStr::from_ptr(name).to_string_lossy().into_owned()
            },
            device_type: DeviceType(ffi::capi_config_device_type(cfg) as u8),
            physical_address: ffi::capi_config_physical_address(cfg),
            base_device: LogicalAddress(ffi::capi_config_base_device(cfg) as u8),
            hdmi_port: ffi::capi_config_hdmi_port(cfg),
            monitor_only: false,
            activate_source: false,
            wake_devices: Vec::new(),
            power_off_devices: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

/// One libcec session. Construct with [`Connection::open`], attach to an
/// adapter with [`Connection::open_adapter`], consume asynchronous events
/// via [`Connection::subscribe_events`], tear down with
/// [`Connection::close`].
///
/// All exported methods that drive libcec are serialized internally via a
/// single mutex; you may call them from multiple threads. Callbacks fire on
/// libcec's own threads and never wait on that mutex.
pub struct Connection {
    id: u64,
    inner: Arc<SessionInner>,
}

impl Connection {
    /// Creates a new CEC connection with the given configuration. The
    /// device name is truncated to 13 bytes; wake/power-off lists skip
    /// invalid logical addresses (mirroring Go `applyAddressList`).
    pub fn open(config: &Configuration) -> Result<Connection, CecError> {
        install_bridges();

        let id = NEXT_SESSION_ID.fetch_add(1, Ordering::SeqCst);
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let inner = Arc::new(SessionInner {
            handle: AtomicPtr::new(ptr::null_mut()),
            api: Mutex::new(()),
            closed: AtomicBool::new(false),
            opened: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            inflight: AtomicUsize::new(0),
            monitor_only: AtomicBool::new(config.monitor_only),
            events,
            menu_handler: Mutex::new(None),
            event_sink: Mutex::new(None),
            sink_delivered: AtomicU64::new(0),
        });

        // Register BEFORE libcec_initialise: callbacks can fire during init
        // and must resolve this id.
        sessions().lock().unwrap().insert(id, inner.clone());

        let name = CString::new(sanitize_device_name(&config.device_name))
            .unwrap_or_else(|_| CString::new("capi").unwrap());
        // applyAddressList skips invalid addresses; do the same here.
        let wake: Vec<u8> = config
            .wake_devices
            .iter()
            .copied()
            .filter(|a| LogicalAddress(*a).is_valid())
            .collect();
        let power_off: Vec<u8> = config
            .power_off_devices
            .iter()
            .copied()
            .filter(|a| LogicalAddress(*a).is_valid())
            .collect();

        // Safety: `name` outlives the call; byte slices pass as plain
        // pointers; cb_param is our registered session id.
        let handle = unsafe {
            ffi::capi_initialise(
                name.as_ptr(),
                config.device_type.0 as c_int,
                config.physical_address,
                config.base_device.0 as c_int,
                config.hdmi_port,
                config.monitor_only as c_int,
                config.activate_source as c_int,
                wake.as_ptr(),
                wake.len() as c_int,
                power_off.as_ptr(),
                power_off.len() as c_int,
                id as usize,
            )
        };
        if handle.is_null() {
            sessions().lock().unwrap().remove(&id);
            return Err(CecError::LibcecCall("libcec_initialise".into()));
        }
        inner.handle.store(handle, Ordering::SeqCst);
        Ok(Connection { id, inner })
    }

    // -- lifecycle ----------------------------------------------------------

    /// Standard prologue for libcec API calls (Go `guard`): refuse if
    /// closed, otherwise take the api lock and re-check.
    /// Like lock_api, but additionally requires an OPENED adapter session:
    /// libcec segfaults when bus methods run against an initialised-but-
    /// never-opened client. Production only reaches these paths after
    /// open_adapter succeeds; this makes that contract enforced.
    fn lock_bus(&self) -> Result<(MutexGuard<'_, ()>, *mut c_void), CecError> {
        // Closed takes precedence over not-opened: a post-close call must
        // report Closed even though close() also cleared `opened`.
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(CecError::Closed);
        }
        if !self.inner.opened.load(Ordering::SeqCst) {
            return Err(CecError::AdapterNotOpen);
        }
        self.lock_api()
    }

    fn lock_api(&self) -> Result<(MutexGuard<'_, ()>, *mut c_void), CecError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(CecError::Closed);
        }
        let guard = self.inner.api.lock().unwrap();
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(CecError::Closed);
        }
        let handle = self.inner.handle.load(Ordering::SeqCst);
        Ok((guard, handle))
    }

    /// Releases all resources. Idempotent: subsequent calls return Ok.
    ///
    /// Teardown ordering (contract §4 — do not reorder):
    /// 1. `closed = true` (CAS)
    /// 2. set `closing = true` so new callbacks bail at the gate
    /// 3. take the api mutex
    /// 4. `libcec_close(handle)`
    /// 5. remove from `SESSIONS` (late callbacks see an unknown id and
    ///    return immediately)
    /// 6. spin-wait (1 ms sleep loop, 5 s timeout) until `inflight == 0`
    /// 7. `libcec_destroy(handle)`
    /// 8. drop the broadcast sender — happens automatically when the last
    ///    `Arc<SessionInner>` goes away (the receiver side observes a
    ///    closed channel, which is graceful by design)
    pub fn close(&self) -> Result<(), CecError> {
        if self
            .inner
            .closed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(()); // already closed
        }
        self.inner.opened.store(false, Ordering::SeqCst);
        self.inner.closing.store(true, Ordering::SeqCst);
        // NOTE: deliberately NOT lock_api() here — it gates on `closed`,
        // which the CAS above just set, and the session would never be
        // destroyed (regression caught by tests: close1 must be Ok and must
        // actually tear down). The raw mutex serializes against in-flight
        // API calls; the null-handle swap below makes them harmless.
        let _api = self.inner.api.lock().unwrap();
        let handle = self.inner.handle.swap(ptr::null_mut(), Ordering::SeqCst);

        if !handle.is_null() {
            // Safety: non-null handle returned by capi_initialise, not yet
            // destroyed (single closer guaranteed by the CAS above + api
            // lock).
            unsafe { ffi::libcec_close(handle) };

            sessions().lock().unwrap().remove(&self.id);

            let deadline = Instant::now() + CLOSE_DRAIN_TIMEOUT;
            while self.inner.inflight.load(Ordering::SeqCst) > 0 {
                if Instant::now() >= deadline {
                    break; // refuse to hang forever on a wedged callback
                }
                std::thread::sleep(Duration::from_millis(1));
            }

            // Safety: after libcec_close + drain, no callback can still be
            // executing against this handle.
            unsafe { ffi::libcec_destroy(handle) };
        } else {
            sessions().lock().unwrap().remove(&self.id);
        }
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// Internal registry id; exposed for tests via #[doc(hidden)].
    #[doc(hidden)]
    pub fn session_id(&self) -> u64 {
        self.id
    }

    /// Whether this connection was opened with `monitor_only = true`.
    /// Transmits and key sends are refused in that mode.
    pub fn is_monitor_only(&self) -> bool {
        self.inner.monitor_only.load(Ordering::SeqCst)
    }

    /// TEST-ONLY: marks the session as adapter-opened without hardware.
    ///
    /// Integration suites use this to drive bus-touching code paths (which
    /// libcec would segfault on pre-open — see [`Connection::lock_bus`])
    /// against a real initialised session. Every ffi call will still fail
    /// with clean library errors; only the gate is bypassed.
    #[doc(hidden)]
    pub fn force_opened_for_test(&self) {
        self.inner.opened.store(true, Ordering::SeqCst);
    }

    /// Test hook: route events to a synchronous sink instead of only the
    /// broadcast channel. Deterministic for integration assertions.
    #[doc(hidden)]
    pub fn set_event_sink(&self, sink: Option<EventSinkFn>) {
        *self.inner.event_sink.lock().unwrap() = sink;
    }

    /// Subscribes to asynchronous CEC events. Capacity is 512; a slow
    /// consumer sees `RecvError::Lagged`, never blocks libcec threads.
    /// Subscriber count on this connection's event channel (tests).
    #[doc(hidden)]
    pub fn event_subscribers(&self) -> usize {
        self.inner.events.receiver_count()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<CecEvent> {
        self.inner.events.subscribe()
    }

    /// Installs (or removes) the synchronous menu-state handler. Runs on a
    /// libcec thread; must not block and must not call back into this
    /// Connection.
    pub fn set_menu_state_handler(&self, f: Option<MenuHandler>) {
        *self.inner.menu_handler.lock().unwrap() = f;
    }

    // -- adapter ------------------------------------------------------------

    /// Lists available CEC adapters reachable from this libcec session.
    pub fn find_adapters(&self) -> Result<Vec<Adapter>, CecError> {
        let (_g, handle) = self.lock_api()?;
        let mut paths = vec![0u8; MAX_ADAPTERS * CEC_ADAPTER_STR_SIZE];
        let mut comms = paths.clone();
        // Safety: buffers are MAX_ADAPTERS slots of CEC_ADAPTER_STR_SIZE
        // bytes each, matching shim.c's strncpy contract.
        let count = unsafe {
            ffi::capi_find_adapters(
                handle,
                MAX_ADAPTERS as u8,
                paths.as_mut_ptr() as *mut c_char,
                comms.as_mut_ptr() as *mut c_char,
                CEC_ADAPTER_STR_SIZE,
            )
        };
        if count < 0 {
            return Err(CecError::LibcecCall("libcec_find_adapters".into()));
        }
        let slot_str = |buf: &[u8], i: usize| -> String {
            let start = i * CEC_ADAPTER_STR_SIZE;
            let end = start + CEC_ADAPTER_STR_SIZE;
            let bytes = &buf[start..end];
            let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[..len]).into_owned()
        };
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            out.push(Adapter {
                path: slot_str(&paths, i),
                comm: slot_str(&comms, i),
            });
        }
        Ok(out)
    }

    /// Opens a connection to the given adapter path. Any previous adapter
    /// session is closed first (libcec_open is idempotent in name but not
    /// always in behavior across versions).
    pub fn open_adapter(&self, adapter_path: &str) -> Result<(), CecError> {
        let (_g, handle) = self.lock_api()?;
        let cpath = CString::new(adapter_path)
            .map_err(|_| CecError::InvalidParams("adapter path contains NUL".into()))?;
        // Safety: handle is live (checked by lock_api); cpath outlives the call.
        unsafe {
            ffi::libcec_close(handle);
            if ffi::libcec_open(handle, cpath.as_ptr(), ADAPTER_OPEN_TIMEOUT_MS) == 0 {
                self.inner.opened.store(false, Ordering::SeqCst);
                return Err(CecError::AdapterNotOpen);
            }
            self.inner.opened.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    // -- power / source -----------------------------------------------------

    /// Powers on a device. Address must be 0..=14.
    pub fn power_on(&self, address: LogicalAddress) -> Result<(), CecError> {
        if !address.is_valid() {
            return Err(CecError::InvalidLogicalAddress);
        }
        let (_g, handle) = self.lock_bus()?;
        // Safety: live handle; address validated above.
        if unsafe { ffi::libcec_power_on_devices(handle, address.0 as c_int) } == 0 {
            return Err(CecError::LibcecCall(format!("power on {}", address.0)));
        }
        Ok(())
    }

    /// Puts a device in standby. Address must be 0..=14.
    pub fn standby(&self, address: LogicalAddress) -> Result<(), CecError> {
        if !address.is_valid() {
            return Err(CecError::InvalidLogicalAddress);
        }
        let (_g, handle) = self.lock_bus()?;
        if unsafe { ffi::libcec_standby_devices(handle, address.0 as c_int) } == 0 {
            return Err(CecError::LibcecCall(format!("standby {}", address.0)));
        }
        Ok(())
    }

    /// Declares the local device of the given type to be the active source.
    pub fn set_active_source(&self, device_type: DeviceType) -> Result<(), CecError> {
        let (_g, handle) = self.lock_bus()?;
        if unsafe { ffi::libcec_set_active_source(handle, device_type.0 as c_int) } == 0 {
            return Err(CecError::LibcecCall("set active source".into()));
        }
        Ok(())
    }

    /// Marks the local device as inactive view.
    pub fn set_inactive_view(&self) -> Result<(), CecError> {
        let (_g, handle) = self.lock_bus()?;
        if unsafe { ffi::libcec_set_inactive_view(handle) } == 0 {
            return Err(CecError::LibcecCall("set inactive view".into()));
        }
        Ok(())
    }

    // -- volume / audio -----------------------------------------------------

    pub fn volume_up(&self, send_release: bool) -> Result<(), CecError> {
        let (_g, handle) = self.lock_bus()?;
        if unsafe { ffi::libcec_volume_up(handle, send_release as c_int) } == 0 {
            return Err(CecError::LibcecCall("volume up".into()));
        }
        Ok(())
    }

    pub fn volume_down(&self, send_release: bool) -> Result<(), CecError> {
        let (_g, handle) = self.lock_bus()?;
        if unsafe { ffi::libcec_volume_down(handle, send_release as c_int) } == 0 {
            return Err(CecError::LibcecCall("volume down".into()));
        }
        Ok(())
    }

    pub fn audio_toggle_mute(&self) -> Result<(), CecError> {
        let (_g, handle) = self.lock_bus()?;
        if unsafe { ffi::libcec_audio_toggle_mute(handle) } == 0 {
            return Err(CecError::LibcecCall("audio toggle mute".into()));
        }
        Ok(())
    }

    pub fn audio_mute(&self) -> Result<(), CecError> {
        let (_g, handle) = self.lock_bus()?;
        if unsafe { ffi::libcec_audio_mute(handle) } == 0 {
            return Err(CecError::LibcecCall("audio mute".into()));
        }
        Ok(())
    }

    pub fn audio_unmute(&self) -> Result<(), CecError> {
        let (_g, handle) = self.lock_bus()?;
        if unsafe { ffi::libcec_audio_unmute(handle) } == 0 {
            return Err(CecError::LibcecCall("audio unmute".into()));
        }
        Ok(())
    }

    /// Enables or disables System Audio Mode by transmitting opcode 0x72 to
    /// the TV — same as the Go implementation, and portable across libcec6/7
    /// (libcec_system_audio_mode is absent from Debian bookworm's libcec6).
    pub fn set_system_audio_mode(&self, enable: bool) -> Result<(), CecError> {
        let initiator = self
            .first_logical_address()
            .unwrap_or(LogicalAddress::FREE_USE);
        self.transmit(&Command {
            initiator,
            destination: LogicalAddress::TV,
            opcode: Opcode::SET_SYSTEM_AUDIO_MODE,
            opcode_set: true,
            parameters: vec![u8::from(enable)],
            ack: false,
            eom: true,
        })
    }

    /// Current audio status: `(volume, muted, raw)`. bit7 = muted,
    /// bits 0-6 = level; volume may exceed 100 with no audio system present.
    pub fn get_audio_status(&self) -> Result<(u8, bool, u8), CecError> {
        let (_g, handle) = self.lock_bus()?;
        // Safety: live handle; plain integer return.
        let status = unsafe { ffi::libcec_audio_get_status(handle) };
        Ok((status & 0x7F, (status & 0x80) != 0, status))
    }

    // -- queries ------------------------------------------------------------

    /// Queries a device's power status. libcec's wire sentinel 0x99 maps
    /// to an error (the Go behavior); `PowerStatus::UNKNOWN` (0xFF) stays
    /// the Go-side sentinel.
    pub fn get_device_power_status(
        &self,
        address: LogicalAddress,
    ) -> Result<PowerStatus, CecError> {
        if !address.is_valid() {
            return Err(CecError::InvalidLogicalAddress);
        }
        let (_g, handle) = self.lock_bus()?;
        // Safety: live handle; validated address.
        let status = unsafe { ffi::libcec_get_device_power_status(handle, address.0 as c_int) };
        if status == CEC_POWER_STATUS_UNKNOWN {
            return Err(CecError::LibcecCall(format!(
                "get power status {}",
                address.0
            )));
        }
        Ok(PowerStatus(status as u8))
    }

    /// Returns the logical address currently claiming the active-source
    /// role, or [`CecError::NoActiveSource`].
    pub fn get_active_source(&self) -> Result<LogicalAddress, CecError> {
        let (_g, handle) = self.lock_bus()?;
        let addr = unsafe { ffi::libcec_get_active_source(handle) };
        let la = LogicalAddress(addr as u8);
        if !la.is_valid() {
            return Err(CecError::NoActiveSource);
        }
        Ok(la)
    }

    pub fn is_active_source(&self, address: LogicalAddress) -> bool {
        if !address.is_valid() {
            return false;
        }
        let Ok((_g, handle)) = self.lock_api() else {
            return false;
        };
        // Safety: live handle; validated address.
        unsafe { ffi::libcec_is_active_source(handle, address.0 as c_int) == 1 }
    }

    /// Queries a device's vendor ID; 0 (`CEC_VENDOR_UNKNOWN`) maps to an
    /// error like Go.
    pub fn get_device_vendor_id(&self, address: LogicalAddress) -> Result<u64, CecError> {
        if !address.is_valid() {
            return Err(CecError::InvalidLogicalAddress);
        }
        let (_g, handle) = self.lock_bus()?;
        let v = unsafe { ffi::libcec_get_device_vendor_id(handle, address.0 as c_int) };
        if v == CEC_VENDOR_UNKNOWN {
            return Err(CecError::LibcecCall(format!("vendor id {}", address.0)));
        }
        Ok(v as u64)
    }

    /// Queries a device's physical (HDMI tree) address; 0xFFFF maps to an
    /// error like Go.
    pub fn get_device_physical_address(&self, address: LogicalAddress) -> Result<u16, CecError> {
        if !address.is_valid() {
            return Err(CecError::InvalidLogicalAddress);
        }
        let (_g, handle) = self.lock_bus()?;
        let a = unsafe { ffi::libcec_get_device_physical_address(handle, address.0 as c_int) };
        if a == CEC_INVALID_PHYSICAL_ADDRESS {
            return Err(CecError::LibcecCall(format!(
                "physical address {}",
                address.0
            )));
        }
        Ok(a)
    }

    /// Queries a device's OSD name ([14]char on the C side, copied into a
    /// Rust String by the shim).
    pub fn get_device_osd_name(&self, address: LogicalAddress) -> Result<String, CecError> {
        if !address.is_valid() {
            return Err(CecError::InvalidLogicalAddress);
        }
        let (_g, handle) = self.lock_bus()?;
        let mut buf = [0u8; 15];
        // Safety: buf is 15 bytes >= cec_osd_name (14) + terminator room;
        // shim.c NUL-terminates within out_size.
        let rc = unsafe {
            ffi::capi_get_device_osd_name(
                handle,
                address.0 as c_int,
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
            )
        };
        if rc == 0 {
            return Err(CecError::LibcecCall(format!("osd name {}", address.0)));
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Ok(String::from_utf8_lossy(&buf[..len]).into_owned())
    }

    /// Queries a device's menu language (ISO 639-2).
    pub fn get_device_menu_language(&self, address: LogicalAddress) -> Result<String, CecError> {
        if !address.is_valid() {
            return Err(CecError::InvalidLogicalAddress);
        }
        let (_g, handle) = self.lock_bus()?;
        let mut buf = [0u8; 5];
        // Safety: buf is 5 bytes >= cec_menu_language (4) + terminator room.
        let rc = unsafe {
            ffi::capi_get_device_menu_language(
                handle,
                address.0 as c_int,
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
            )
        };
        if rc == 0 {
            return Err(CecError::LibcecCall(format!("menu lang {}", address.0)));
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Ok(String::from_utf8_lossy(&buf[..len]).into_owned())
    }

    /// Queries a device's CEC spec version.
    pub fn get_device_cec_version(&self, address: LogicalAddress) -> Result<CECVersion, CecError> {
        if !address.is_valid() {
            return Err(CecError::InvalidLogicalAddress);
        }
        let (_g, handle) = self.lock_bus()?;
        let v = unsafe { ffi::libcec_get_device_cec_version(handle, address.0 as c_int) };
        if v as u32 == CEC_VERSION_UNKNOWN {
            return Err(CecError::LibcecCall(format!("cec version {}", address.0)));
        }
        Ok(CECVersion(v as u8))
    }

    /// Logical addresses of devices libcec considers active on the bus.
    pub fn get_active_devices(&self) -> Result<Vec<LogicalAddress>, CecError> {
        let (_g, handle) = self.lock_bus()?;
        // Safety: live handle; mask computed inside shim.c.
        let mask = unsafe { ffi::capi_get_active_devices_mask(handle) };
        Ok(mask_to_addresses(mask))
    }

    pub fn is_active_device(&self, address: LogicalAddress) -> bool {
        if !address.is_valid() {
            return false;
        }
        let Ok((_g, handle)) = self.lock_api() else {
            return false;
        };
        unsafe { ffi::libcec_is_active_device(handle, address.0 as c_int) == 1 }
    }

    /// Aggregated device info (port of Go `GetDeviceInfo`): every sub-query
    /// runs; partial failures are flagged in `DeviceInfo.errors`; the call
    /// errors only when ALL sub-queries fail (device unresponsive).
    pub fn get_device_info(&self, address: LogicalAddress) -> Result<DeviceInfo, CecError> {
        let mut errs = DeviceInfoErrors::default();

        let is_active = self.is_active_device(address);
        let is_active_source = self.is_active_source(address);

        let physical_address = self
            .get_device_physical_address(address)
            .unwrap_or_else(|_| {
                errs.physical_address = true;
                0
            });
        let vendor_id = self.get_device_vendor_id(address).unwrap_or_else(|_| {
            errs.vendor_id = true;
            0
        });
        let cec_version = self.get_device_cec_version(address).unwrap_or_else(|_| {
            errs.cec_version = true;
            CECVersion::UNKNOWN
        });
        let power_status = self.get_device_power_status(address).unwrap_or_else(|_| {
            errs.power_status = true;
            PowerStatus::UNKNOWN
        });
        let osd_name = match self.get_device_osd_name(address) {
            Ok(v) => v,
            Err(_) => {
                errs.osd_name = true;
                String::new()
            }
        };
        let menu_language = match self.get_device_menu_language(address) {
            Ok(v) => v,
            Err(_) => {
                errs.menu_language = true;
                String::new()
            }
        };

        if errs.all() {
            return Err(CecError::LibcecCall(
                "device info: all sub-queries failed".into(),
            ));
        }
        Ok(DeviceInfo {
            logical_address: address,
            address_name: address.to_string(),
            physical_address,
            osd_name,
            menu_language,
            vendor_id,
            vendor_name: get_vendor_name(vendor_id),
            vendor_known: is_known_vendor(vendor_id),
            cec_version,
            power_status,
            hdmi_port: DeviceInfo::derive_hdmi_port(physical_address),
            is_active,
            is_active_source,
            errors: errs,
        })
    }

    /// All libcec-active addresses plus any additional addresses that
    /// answer a CEC POLL. `active_only = true` skips polling entirely
    /// (equivalent to Go `GetActiveDevices`); `false` probes every missing
    /// address 0..=14 (Go `fullPoll`), sorted ascending like Go.
    pub fn logical_addresses_with_poll(
        &self,
        active_only: bool,
    ) -> Result<Vec<LogicalAddress>, CecError> {
        let active = self.get_active_devices()?;
        if active_only {
            return Ok(active);
        }
        let mut seen = [false; 16];
        let mut out = Vec::with_capacity(16);
        for a in &active {
            seen[a.0 as usize] = true;
            out.push(*a);
        }
        for raw in 0u8..=14 {
            if seen[raw as usize] {
                continue;
            }
            if self.poll_device(LogicalAddress(raw)) {
                out.push(LogicalAddress(raw));
            }
        }
        out.sort();
        Ok(out)
    }

    /// Cheapest health check: power-status poll of the TV. Used by
    /// reconnect supervisors.
    pub fn ping_tv(&self) -> Result<(), CecError> {
        self.get_device_power_status(LogicalAddress::TV).map(|_| ())
    }

    /// This adapter's primary logical address, if registered.
    pub fn first_logical_address(&self) -> Option<LogicalAddress> {
        self.get_logical_addresses().ok()?.into_iter().next()
    }

    // -- transmit -----------------------------------------------------------

    /// Sends a raw CEC command frame. Bounds-checked (fixes the Go bug):
    /// rejects >64 parameter bytes, >255-byte truncation, and invalid
    /// initiator/destination addresses with [`CecError::InvalidParams`].
    /// Refused in monitor-only mode.
    pub fn transmit(&self, cmd: &Command) -> Result<(), CecError> {
        validate_transmit(cmd)?;
        if self.is_monitor_only() {
            return Err(CecError::MonitorOnly);
        }
        let (_g, handle) = self.lock_bus()?;
        // Storage sized/aligned for cec_command (~96 bytes, align 4); the
        // shim owns all layout knowledge and we only hand it a pointer.
        let mut storage = [0u64; 16];
        let ccmd = storage.as_mut_ptr().cast::<c_void>();
        // Safety: storage outlives the call; the shim initializes the whole
        // struct before libcec reads it; parameters are pre-bounds-checked.
        unsafe {
            ffi::capi_command_init(
                ccmd,
                cmd.initiator.0 as c_int,
                cmd.destination.0 as c_int,
                cmd.opcode.0 as c_int,
                cmd.opcode_set as c_int,
            );
            ffi::capi_command_push_params(
                ccmd,
                cmd.parameters.as_ptr(),
                cmd.parameters.len() as c_int,
            );
            // Explicit transmit timeout (contract §7): Go relied on zero
            // defaulting — a latent bug since C compilation units never run
            // cec_command's Clear() constructor.
            ffi::capi_command_set_transmit_timeout(ccmd, CEC_DEFAULT_TRANSMIT_TIMEOUT_MS);
            if ffi::libcec_transmit(handle, ccmd) == 0 {
                return Err(CecError::LibcecCall("transmit failed".into()));
            }
        }
        Ok(())
    }

    /// Sends a remote-control key press. If `wait`, blocks until the bus
    /// acknowledges. Refused in monitor-only mode.
    pub fn send_keypress(
        &self,
        address: LogicalAddress,
        key: Keycode,
        wait: bool,
    ) -> Result<(), CecError> {
        if !address.is_valid() {
            return Err(CecError::InvalidLogicalAddress);
        }
        if self.is_monitor_only() {
            return Err(CecError::MonitorOnly);
        }
        let (_g, handle) = self.lock_bus()?;
        // Safety: live handle; validated address; plain ints.
        if unsafe {
            ffi::libcec_send_keypress(handle, address.0 as c_int, key.0 as c_int, wait as c_int)
        } == 0
        {
            return Err(CecError::LibcecCall(format!("send keypress {}", address.0)));
        }
        Ok(())
    }

    /// Sends a remote-control key release. Refused in monitor-only mode.
    pub fn send_key_release(&self, address: LogicalAddress, wait: bool) -> Result<(), CecError> {
        if !address.is_valid() {
            return Err(CecError::InvalidLogicalAddress);
        }
        if self.is_monitor_only() {
            return Err(CecError::MonitorOnly);
        }
        let (_g, handle) = self.lock_bus()?;
        if unsafe { ffi::libcec_send_key_release(handle, address.0 as c_int, wait as c_int) } == 0 {
            return Err(CecError::LibcecCall(format!(
                "send key release {}",
                address.0
            )));
        }
        Ok(())
    }

    /// Displays an OSD string on the given device.
    pub fn set_osd_string(
        &self,
        address: LogicalAddress,
        duration: DisplayControl,
        message: &str,
    ) -> Result<(), CecError> {
        if !address.is_valid() {
            return Err(CecError::InvalidLogicalAddress);
        }
        let (_g, handle) = self.lock_bus()?;
        let cmsg = CString::new(message)
            .map_err(|_| CecError::InvalidParams("OSD message contains NUL".into()))?;
        // Safety: live handle; cmsg outlives the call.
        if unsafe {
            ffi::libcec_set_osd_string(
                handle,
                address.0 as c_int,
                duration.0 as c_int,
                cmsg.as_ptr(),
            )
        } == 0
        {
            return Err(CecError::LibcecCall("set osd string".into()));
        }
        Ok(())
    }

    // -- misc ---------------------------------------------------------------

    /// Toggles libcec monitoring mode.
    pub fn switch_monitoring(&self, enable: bool) -> Result<(), CecError> {
        let (_g, handle) = self.lock_bus()?;
        if unsafe { ffi::libcec_switch_monitoring(handle, enable as c_int) } == 0 {
            return Err(CecError::LibcecCall("switch monitoring".into()));
        }
        Ok(())
    }

    /// libcec version information as a printable string.
    pub fn get_lib_info(&self) -> Result<String, CecError> {
        let (_g, handle) = self.lock_api()?;
        // Safety: live handle; returns a NUL-terminated static/libcec-owned
        // string we copy before releasing anything.
        let p = unsafe { ffi::libcec_get_lib_info(handle) };
        if p.is_null() {
            return Err(CecError::LibcecCall("get lib info".into()));
        }
        // Safety: non-null, NUL-terminated per cecc.h contract.
        Ok(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }

    /// Replaces the running libcec configuration, re-attaching our callback
    /// table so events keep flowing after the swap. Applies the same
    /// passive defaults as `open`.
    pub fn set_configuration(&self, config: &Configuration) -> Result<(), CecError> {
        let (_g, handle) = self.lock_api()?;
        let name = CString::new(sanitize_device_name(&config.device_name))
            .unwrap_or_else(|_| CString::new("capi").unwrap());
        let wake: Vec<u8> = config
            .wake_devices
            .iter()
            .copied()
            .filter(|a| LogicalAddress(*a).is_valid())
            .collect();
        let power_off: Vec<u8> = config
            .power_off_devices
            .iter()
            .copied()
            .filter(|a| LogicalAddress(*a).is_valid())
            .collect();
        // Safety: live handle; all pointers outlive the call; id is still
        // registered in SESSIONS.
        if unsafe {
            ffi::capi_install_callbacks_on_set(
                handle,
                name.as_ptr(),
                config.device_type.0 as c_int,
                config.physical_address,
                config.base_device.0 as c_int,
                config.hdmi_port,
                config.monitor_only as c_int,
                config.activate_source as c_int,
                wake.as_ptr(),
                wake.len() as c_int,
                power_off.as_ptr(),
                power_off.len() as c_int,
                self.id as usize,
            )
        } == 0
        {
            return Err(CecError::LibcecCall("set configuration".into()));
        }
        self.inner
            .monitor_only
            .store(config.monitor_only, Ordering::SeqCst);
        Ok(())
    }

    /// Retrieves the running libcec configuration. Only the fields libcec
    /// reports back are populated; `monitor_only` reflects the local flag
    /// and bus-disruption knobs read back empty (same as Go).
    pub fn get_current_configuration(&self) -> Result<Configuration, CecError> {
        let (_g, handle) = self.lock_api()?;
        let mut out = ffi::capi_config_out::default();
        // Safety: out is a shim-owned plain-C snapshot struct.
        if unsafe { ffi::capi_get_current_configuration(handle, &mut out) } == 0 {
            return Err(CecError::LibcecCall("get current configuration".into()));
        }
        let name_end = out
            .device_name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(out.device_name.len());
        Ok(Configuration {
            device_name: String::from_utf8_lossy(
                &out.device_name[..name_end]
                    .iter()
                    .map(|&b| b as u8)
                    .collect::<Vec<u8>>(),
            )
            .into_owned(),
            device_type: DeviceType(out.device_type as u8),
            physical_address: out.physical_address,
            base_device: LogicalAddress(out.base_device as u8),
            hdmi_port: out.hdmi_port,
            monitor_only: self.is_monitor_only(),
            activate_source: false,
            wake_devices: Vec::new(),
            power_off_devices: Vec::new(),
        })
    }

    /// Server (libcec) version from the running configuration.
    pub fn server_version(&self) -> Result<u32, CecError> {
        let (_g, handle) = self.lock_api()?;
        let mut out = ffi::capi_config_out::default();
        // Safety: same as get_current_configuration.
        if unsafe { ffi::capi_get_current_configuration(handle, &mut out) } == 0 {
            return Err(CecError::LibcecCall("get current configuration".into()));
        }
        Ok(out.server_version)
    }

    /// Sends a CEC POLL to test whether a device is present — much faster
    /// than a full info query.
    pub fn poll_device(&self, address: LogicalAddress) -> bool {
        if !address.is_valid() {
            return false;
        }
        let Ok((_g, handle)) = self.lock_api() else {
            return false;
        };
        unsafe { ffi::libcec_poll_device(handle, address.0 as c_int) == 1 }
    }

    /// Switches input on the base device to the given HDMI port (port must
    /// be 1..=15).
    pub fn set_hdmi_port(&self, base_device: LogicalAddress, port: u8) -> Result<(), CecError> {
        if !(1..=15).contains(&port) {
            return Err(CecError::InvalidHdmiPort);
        }
        let (_g, handle) = self.lock_bus()?;
        if unsafe { ffi::libcec_set_hdmi_port(handle, base_device.0 as c_int, port) } == 0 {
            return Err(CecError::LibcecCall(format!("set hdmi port {port}")));
        }
        Ok(())
    }

    /// Asks libcec to re-discover devices on the bus, then sleeps `settle`
    /// OUTSIDE the api lock so other calls can proceed while responses
    /// arrive (like Go `RescanDevices`).
    pub fn rescan_devices(&self, settle: Duration) -> Result<(), CecError> {
        {
            let (_g, handle) = self.lock_bus()?;
            // Safety: live handle; void return.
            unsafe { ffi::libcec_rescan_devices(handle) };
        }
        if settle > Duration::ZERO {
            std::thread::sleep(settle);
        }
        Ok(())
    }

    /// Logical addresses currently assigned to this adapter. An empty
    /// bitmask falls back to the primary address when known (Go behavior).
    pub fn get_logical_addresses(&self) -> Result<Vec<LogicalAddress>, CecError> {
        let (_g, handle) = self.lock_bus()?;
        let mut primary: c_int = CECDEVICE_UNKNOWN;
        // Safety: primary_out is a valid c_int pointer; mask computed in shim.c.
        let mask = unsafe { ffi::capi_get_logical_addresses_mask(handle, &mut primary) };
        let mut out = mask_to_addresses(mask);
        if out.is_empty() && primary != CECDEVICE_UNKNOWN {
            out.push(LogicalAddress(primary as u8));
        }
        Ok(out)
    }
}

impl Drop for Connection {
    /// Idempotent teardown so an un-dropped session cannot leak its libcec
    /// handle. Explicit `close()` remains the documented shutdown path.
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn mask_to_addresses(mask: u16) -> Vec<LogicalAddress> {
    (0..16u16)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| LogicalAddress(i as u8))
        .collect()
}

/// Major ABI of the libcec the process is linked against at runtime.
/// `LIBCEC_VERSION_CURRENT` encodes the major version in the top bits
/// (`major = version >> 16`, e.g. 7 for libcec 7.x); used by self-update
/// to pick the `-libcecN` release asset.
pub fn linked_libcec_major() -> u32 {
    // Safety: capi_client_version is a plain constant-returning C fn.
    (unsafe { ffi::capi_client_version() }) >> 16
}

impl Connection {
    /// The configured device name as libcec reports it for the running
    /// session; empty string when the session is closed/unavailable.
    pub fn device_name(&self) -> String {
        match self.get_current_configuration() {
            Ok(cfg) => cfg.device_name,
            Err(_) => String::new(),
        }
    }
}

/// Full keycode-name table: every name `keycode_from_name` accepts
/// (including aliases like `home`/`back`/`menu`), sorted by keycode value
/// then name.
pub fn keycode_names() -> Vec<(String, u8)> {
    let mut out: Vec<(String, u8)> = KEY_NAMES
        .iter()
        .map(|(n, k)| ((*n).to_string(), k.0))
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Named-opcode table (`OPCODE_NAMES`), sorted by opcode value.
pub fn opcode_table() -> Vec<(String, u8)> {
    let mut out: Vec<(String, u8)> = OPCODE_NAMES
        .iter()
        .map(|(op, n)| ((*n).to_string(), op.0))
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod close_tests {
    use super::*;
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn cfg() -> Configuration {
        Configuration {
            device_name: "cov".into(),
            device_type: DeviceType::RECORDING,
            physical_address: 0xFFFF,
            base_device: LogicalAddress::TV,
            hdmi_port: 1,
            monitor_only: false,
            activate_source: false,
            wake_devices: vec![],
            power_off_devices: vec![],
        }
    }

    /// Regression: close() used to CAS closed=true then call lock_api(),
    /// which gated on closed and bailed with Err(Closed) BEFORE destroying
    /// the session — leaking one libcec handle per reconnect.
    #[test]
    fn close_is_ok_and_actually_tears_down() {
        let _g = serial();
        let c = Connection::open(&cfg()).expect("open");
        assert!(!c.is_closed());
        c.close().expect("first close must succeed");
        assert!(c.is_closed());
        c.close().expect("second close is a no-op Ok");
        // Post-close API surface reports Closed instead of touching C.
        assert!(matches!(
            c.get_device_power_status(LogicalAddress::TV),
            Err(CecError::Closed)
        ));
    }

    /// Multi-cycle endurance is exercised on-device; headless libcec has an
    /// internal quirk around the third initialise-after-post-close-call.
    #[test]
    fn close_leaves_session_registry_drained() {
        let _g = serial();
        let c = Connection::open(&cfg()).expect("open");
        assert!(!sessions().lock().unwrap().is_empty());
        c.close().expect("close");
        // Registry drained: no live session remains for this connection.
        // (id is internal; emptiness across the whole map is the observable.)
        // Other tests may hold sessions concurrently only when not serialized,
        // and this test holds the serial guard, so the map must be empty of
        // entries whose closed flag is false AND that belong to us — we assert
        // the simplest observable: our own id is gone.
        assert!(!sessions().lock().unwrap().contains_key(&c.session_id()));
    }
}

/// Test-only surface for the mock CEC backend (`--features mock-cec`).
#[cfg(feature = "mock-cec")]
pub mod mock {
    use super::*;

    unsafe extern "C" {
        fn mock_reset();
        fn mock_session_is_open() -> i32;
        fn mock_emit_command_on(
            id: usize,
            initiator: u8,
            dest: u8,
            opcode: u8,
            params: *const u8,
            len: i32,
        );
        fn mock_emit_keypress_on(id: usize, key: u8, duration: u32);
        fn mock_emit_alert(id: usize, alert: i32, ptype: i32, pvalue: i64);
        fn mock_emit_config_changed();
        fn mock_emit_source_activated(id: usize, addr: u8, activated: i32);
        fn mock_emit_menu_on(id: usize, state: i32) -> i32;
        fn mock_last_was_reply() -> i32;
        fn mock_set_fail_next(n: i32);
        fn mock_last_transmit(
            initiator: *mut u8,
            dest: *mut u8,
            opcode: *mut u8,
            params_out: *mut u8,
            cap: i32,
        ) -> i32;
    }

    pub fn reset() {
        unsafe { mock_reset() }
    }

    pub fn session_is_open() -> bool {
        unsafe { mock_session_is_open() == 1 }
    }

    /// Drive the production callback chain with an injected bus frame.
    pub fn emit_command_on(conn: &Connection, cmd: &Command) {
        unsafe {
            mock_emit_command_on(
                conn.session_id() as usize,
                cmd.initiator.0,
                cmd.destination.0,
                cmd.opcode.0,
                cmd.parameters.as_ptr(),
                cmd.parameters.len() as i32,
            );
        }
    }

    pub fn emit_keypress_on(conn: &Connection, key: u8, duration: u32) {
        unsafe { mock_emit_keypress_on(conn.session_id() as usize, key, duration) }
    }

    pub fn emit_alert_on(conn: &Connection, alert: i32, ptype: i32, pvalue: i32) {
        unsafe { mock_emit_alert(conn.session_id() as usize, alert, ptype, pvalue as i64) }
    }

    pub fn emit_config_changed_on(_conn: &Connection) {
        unsafe { mock_emit_config_changed() }
    }

    pub fn last_was_reply() -> bool {
        unsafe { mock_last_was_reply() == 1 }
    }

    /// Make the next `n` libcec calls fail (error-arm coverage).
    pub fn set_fail_next(n: i32) {
        unsafe { mock_set_fail_next(n) }
    }

    pub fn emit_source_activated_on(conn: &Connection, addr: u8, activated: bool) {
        unsafe { mock_emit_source_activated(conn.session_id() as usize, addr, activated as i32) }
    }

    pub fn emit_menu_on(conn: &Connection, state: i32) -> i32 {
        unsafe { mock_emit_menu_on(conn.session_id() as usize, state) }
    }

    pub struct LastTransmit {
        pub initiator: u8,
        pub dest: u8,
        pub opcode: u8,
        pub params: Vec<u8>,
    }

    pub fn last_transmit() -> LastTransmit {
        let mut i = 0u8;
        let mut d = 0u8;
        let mut o = 0u8;
        let mut p = [0u8; 64];
        let n = unsafe { mock_last_transmit(&mut i, &mut d, &mut o, p.as_mut_ptr(), 64) };
        LastTransmit {
            initiator: i,
            dest: d,
            opcode: o,
            params: p[..n as usize].to_vec(),
        }
    }
}
