// C-side libcec callback thunks. These run on libcec's internal threads and
// call into Go via the //export bridges in bridges.go. The first argument is
// always the cbParam value we set in libcec_configuration.callbackParam, which
// is a runtime/cgo.Handle uintptr identifying the Go *Connection.

#include <libcec/cecc.h>
#include <stdint.h>

extern void cec_bridge_log(uintptr_t handle, int level, int64_t timestamp, const char* message);
extern void cec_bridge_key(uintptr_t handle, int keycode, unsigned int duration);
extern void cec_bridge_command(uintptr_t handle, const cec_command* command);
extern void cec_bridge_config(uintptr_t handle, const libcec_configuration* config);
extern void cec_bridge_alert(uintptr_t handle, int alert, int paramType, int64_t paramValue);
extern int  cec_bridge_menu(uintptr_t handle, int state);
extern void cec_bridge_source(uintptr_t handle, int address, int activated);

static void cec_c_log(void* cbParam, const cec_log_message* msg) {
    if (!msg || !msg->message) return;
    cec_bridge_log((uintptr_t)cbParam, msg->level, msg->time, msg->message);
}

static void cec_c_key(void* cbParam, const cec_keypress* key) {
    if (!key) return;
    cec_bridge_key((uintptr_t)cbParam, key->keycode, key->duration);
}

static void cec_c_command(void* cbParam, const cec_command* cmd) {
    if (!cmd) return;
    cec_bridge_command((uintptr_t)cbParam, cmd);
}

static void cec_c_config(void* cbParam, const libcec_configuration* cfg) {
    if (!cfg) return;
    cec_bridge_config((uintptr_t)cbParam, cfg);
}

static void cec_c_alert(void* cbParam, const libcec_alert alert, const libcec_parameter param) {
    cec_bridge_alert((uintptr_t)cbParam, (int)alert,
                     (int)param.paramType, (int64_t)(intptr_t)param.paramData);
}

static int cec_c_menu(void* cbParam, const cec_menu_state state) {
    return cec_bridge_menu((uintptr_t)cbParam, (int)state);
}

static void cec_c_source(void* cbParam, const cec_logical_address addr, const uint8_t activated) {
    cec_bridge_source((uintptr_t)cbParam, (int)addr, (int)activated);
}

// cec_install_callbacks fills in the callbacks pointer and callbackParam of
// a libcec_configuration with the process-wide callback table and the given
// cgo.Handle. This avoids the (correct but vet-flagged) Go pattern of
// converting uintptr to unsafe.Pointer at the call site.
void cec_install_callbacks(libcec_configuration* cfg, uintptr_t handle);

// cec_callback_table returns a pointer to a process-wide callback table.
// The table is identical for every connection; the per-connection identity
// is carried in libcec_configuration.callbackParam (a cgo.Handle).
ICECCallbacks* cec_callback_table(void) {
    static ICECCallbacks tbl;
    static int initialized = 0;
    if (!initialized) {
        tbl.logMessage           = cec_c_log;
        tbl.keyPress             = cec_c_key;
        tbl.commandReceived      = cec_c_command;
        tbl.configurationChanged = cec_c_config;
        tbl.alert                = cec_c_alert;
        tbl.menuStateChanged     = cec_c_menu;
        tbl.sourceActivated      = cec_c_source;
        initialized = 1;
    }
    return &tbl;
}

// cec_command_param_byte reads parameters.data[i] without exposing the
// flexible-array struct layout to Go directly.
uint8_t cec_command_param_byte(const cec_command* cmd, int i) {
    return cmd->parameters.data[i];
}

uint8_t cec_command_param_size(const cec_command* cmd) {
    return cmd->parameters.size;
}

void cec_install_callbacks(libcec_configuration* cfg, uintptr_t handle) {
    cfg->callbacks     = cec_callback_table();
    cfg->callbackParam = (void*)handle;
}

// cec_apply_address_list clears a cec_logical_addresses struct and then
// populates it with up to 16 entries. Used to override the bus-disrupting
// libcec defaults (wakeDevices = TV, powerOffDevices = BROADCAST) without
// reaching into the cgo struct layout from Go.
void cec_apply_address_list(cec_logical_addresses* dest, const uint8_t* addrs, int n) {
    if (!dest) return;
    dest->primary = CECDEVICE_UNKNOWN;
    for (int i = 0; i < 16; i++) {
        dest->addresses[i] = 0;
    }
    if (!addrs || n <= 0) return;
    for (int i = 0; i < n; i++) {
        uint8_t la = addrs[i];
        if (la > 15) continue;
        if (i == 0) dest->primary = (cec_logical_address)la;
        dest->addresses[la] = 1;
    }
}

// cec_set_passive_defaults zeros out every libcec configuration knob that
// would otherwise emit state-changing CEC traffic on libcec_open or
// libcec_destroy. Callers re-enable individual knobs after this if they want
// the default behavior back.
void cec_set_passive_defaults(libcec_configuration* cfg) {
    if (!cfg) return;
    cfg->bActivateSource = 0;
    cfg->bMonitorOnly    = 0;
    cec_apply_address_list(&cfg->wakeDevices,     NULL, 0);
    cec_apply_address_list(&cfg->powerOffDevices, NULL, 0);
}

void cec_set_activate_source(libcec_configuration* cfg, int v) {
    if (cfg) cfg->bActivateSource = (uint8_t)(v ? 1 : 0);
}

void cec_set_monitor_only(libcec_configuration* cfg, int v) {
    if (cfg) cfg->bMonitorOnly = (uint8_t)(v ? 1 : 0);
}
