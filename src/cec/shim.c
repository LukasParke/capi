/*
 * C shim between Rust (src/cec/ffi.rs) and libcec's flat cecc.h API.
 *
 * Mirrors cec/bridges_c.c from the Go port: every access to a libcec struct
 * (libcec_configuration, cec_command, cec_logical_addresses, cec_adapter,
 * cec_osd_name, cec_menu_language) lives here so Rust never replicates a
 * libcec struct layout. This keeps us immune to libcec6 vs libcec7 drift.
 *
 * Callback thunks run on libcec's internal threads. They call plain function
 * pointers from the process-wide `capi_bridges` table (installed once from
 * Rust via capi_set_bridges) and pass the session id as uintptr_t cbParam.
 */
#include <libcec/cecc.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* ------------------------------------------------------------------ */
/* Process-wide bridge table, filled once from Rust at startup.        */
/* ------------------------------------------------------------------ */

typedef struct capi_bridges {
    void (*log)(uintptr_t id, int32_t level, int64_t time, const char *message);
    void (*key)(uintptr_t id, int keycode, unsigned duration);
    void (*command)(uintptr_t id, const cec_command *cmd);
    void (*config_changed)(uintptr_t id, const libcec_configuration *cfg);
    void (*alert)(uintptr_t id, int alert, int param_type, int64_t param_value);
    int (*menu)(uintptr_t id, int state);
    void (*source)(uintptr_t id, int address, int activated);
} capi_bridges;

static capi_bridges g_bridges;

void capi_set_bridges(const capi_bridges *bridges) {
    if (bridges)
        g_bridges = *bridges;
}

uint32_t capi_client_version(void) {
    return (uint32_t)LIBCEC_VERSION_CURRENT;
}

/* ------------------------------------------------------------------ */
/* Callback thunks (port of cec/bridges_c.c).                          */
/* ------------------------------------------------------------------ */

static void cec_c_log(void *cbParam, const cec_log_message *msg) {
    if (!g_bridges.log || !msg || !msg->message)
        return;
    g_bridges.log((uintptr_t)cbParam, (int32_t)msg->level, (int64_t)msg->time, msg->message);
}

static void cec_c_key(void *cbParam, const cec_keypress *key) {
    if (!g_bridges.key || !key)
        return;
    g_bridges.key((uintptr_t)cbParam, (int)key->keycode, (unsigned)key->duration);
}

static void cec_c_command(void *cbParam, const cec_command *cmd) {
    if (!g_bridges.command || !cmd)
        return;
    g_bridges.command((uintptr_t)cbParam, cmd);
}

static void cec_c_config(void *cbParam, const libcec_configuration *cfg) {
    if (!g_bridges.config_changed || !cfg)
        return;
    g_bridges.config_changed((uintptr_t)cbParam, cfg);
}

static void cec_c_alert(void *cbParam, const libcec_alert alert, const libcec_parameter param) {
    if (!g_bridges.alert)
        return;
    g_bridges.alert((uintptr_t)cbParam, (int)alert, (int)param.paramType,
                    (int64_t)(intptr_t)param.paramData);
}

static int cec_c_menu(void *cbParam, const cec_menu_state state) {
    if (!g_bridges.menu)
        return 1;
    return g_bridges.menu((uintptr_t)cbParam, (int)state);
}

static void cec_c_source(void *cbParam, const cec_logical_address addr, const uint8_t activated) {
    if (!g_bridges.source)
        return;
    g_bridges.source((uintptr_t)cbParam, (int)addr, (int)activated);
}

/* Process-wide ICECCallbacks table; identical for every connection. The
 * per-session identity rides in libcec_configuration.callbackParam. */
ICECCallbacks *capi_callback_table(void) {
    static ICECCallbacks tbl;
    static int initialized = 0;
    if (!initialized) {
        memset(&tbl, 0, sizeof(tbl));
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

void capi_install_callbacks(libcec_configuration *cfg, uintptr_t id) {
    if (!cfg)
        return;
    cfg->callbacks     = capi_callback_table();
    cfg->callbackParam = (void *)(uintptr_t)id;
}

/* ------------------------------------------------------------------ */
/* Configuration helpers (port of cec_set_passive_defaults et al).      */
/* ------------------------------------------------------------------ */

/* Clears dest and populates it with up to 16 entries. An empty/NULL list
 * clears it, suppressing libcec's bus-disrupting defaults
 * (wakeDevices={TV}, powerOffDevices={BROADCAST}). */
void capi_apply_address_list(cec_logical_addresses *dest, const uint8_t *addrs, int n) {
    if (!dest)
        return;
    dest->primary = CECDEVICE_UNKNOWN;
    for (int i = 0; i < 16; i++)
        dest->addresses[i] = 0;
    if (!addrs || n <= 0)
        return;
    for (int i = 0; i < n; i++) {
        uint8_t la = addrs[i];
        if (la > 15)
            continue;
        if (i == 0)
            dest->primary = (cec_logical_address)la;
        dest->addresses[la] = 1;
    }
}

void capi_set_passive_defaults(libcec_configuration *cfg) {
    if (!cfg)
        return;
    cfg->bActivateSource = 0;
    cfg->bMonitorOnly    = 0;
    capi_apply_address_list(&cfg->wakeDevices, NULL, 0);
    capi_apply_address_list(&cfg->powerOffDevices, NULL, 0);
}

/* Builds a heap-allocated libcec_configuration from flat arguments:
 * zeroed via libcec_clear_configuration, passive defaults applied, then
 * caller overrides. Returns NULL on allocation failure. Free with
 * capi_free_config. */
libcec_configuration *capi_build_config(const char *device_name, int device_type,
                                        uint16_t physical_address, int base_device,
                                        uint8_t hdmi_port, uint32_t client_version,
                                        int monitor_only, int activate_source,
                                        const uint8_t *wake, int wake_n,
                                        const uint8_t *poweroff, int poweroff_n) {
    libcec_configuration *cfg = calloc(1, sizeof(*cfg));
    if (!cfg)
        return NULL;
    libcec_clear_configuration(cfg);
    capi_set_passive_defaults(cfg);

    if (device_name) {
        strncpy(cfg->strDeviceName, device_name, LIBCEC_OSD_NAME_SIZE - 1);
        cfg->strDeviceName[LIBCEC_OSD_NAME_SIZE - 1] = '\0';
    }
    cfg->deviceTypes.types[0] = (cec_device_type)device_type;
    cfg->iPhysicalAddress     = physical_address;
    cfg->baseDevice           = (cec_logical_address)base_device;
    cfg->iHDMIPort            = hdmi_port;
    cfg->clientVersion        = client_version;

    if (monitor_only)
        cfg->bMonitorOnly = 1;
    if (activate_source)
        cfg->bActivateSource = 1;

    capi_apply_address_list(&cfg->wakeDevices, wake, wake_n);
    capi_apply_address_list(&cfg->powerOffDevices, poweroff, poweroff_n);
    return cfg;
}

void capi_free_config(libcec_configuration *cfg) {
    free(cfg);
}

void *capi_initialise(const char *device_name, int device_type,
                      uint16_t physical_address, int base_device, uint8_t hdmi_port,
                      int monitor_only, int activate_source,
                      const uint8_t *wake, int wake_n,
                      const uint8_t *poweroff, int poweroff_n,
                      uintptr_t cb_param) {
    libcec_configuration *cfg = capi_build_config(device_name, device_type,
                                                  physical_address, base_device,
                                                  hdmi_port, capi_client_version(),
                                                  monitor_only, activate_source,
                                                  wake, wake_n, poweroff, poweroff_n);
    if (!cfg)
        return NULL;
    capi_install_callbacks(cfg, cb_param);
    libcec_connection_t handle = libcec_initialise(cfg);
    capi_free_config(cfg);
    return (void *)handle;
}

/* Rebuilds the configuration and hands it to the running session via
 * libcec_set_configuration, re-attaching our callback table so events keep
 * flowing after the swap. Returns 0 on failure (libcec contract). */
int capi_install_callbacks_on_set(void *handle, const char *device_name, int device_type,
                                  uint16_t physical_address, int base_device, uint8_t hdmi_port,
                                  int monitor_only, int activate_source,
                                  const uint8_t *wake, int wake_n,
                                  const uint8_t *poweroff, int poweroff_n,
                                  uintptr_t cb_param) {
    libcec_configuration *cfg = capi_build_config(device_name, device_type,
                                                  physical_address, base_device,
                                                  hdmi_port, capi_client_version(),
                                                  monitor_only, activate_source,
                                                  wake, wake_n, poweroff, poweroff_n);
    if (!cfg)
        return 0;
    capi_install_callbacks(cfg, cb_param);
    int rc = libcec_set_configuration((libcec_connection_t)handle, cfg);
    capi_free_config(cfg);
    return rc;
}

/* ------------------------------------------------------------------ */
/* cec_command helpers.                                                */
/* ------------------------------------------------------------------ */

uint8_t capi_command_param_byte(const cec_command *cmd, int i) {
    return cmd->parameters.data[i];
}

uint8_t capi_command_param_size(const cec_command *cmd) {
    return cmd->parameters.size;
}

/* Initializes a command frame and sets an explicit transmit timeout.
 * In C compilation units cec_command's Clear() constructor never runs, so
 * transmit_timeout would stay 0 (the latent Go bug); we set it explicitly. */
void capi_command_init(cec_command *cmd, int initiator, int destination,
                       int opcode, int opcode_set) {
    if (!cmd)
        return;
    memset(cmd, 0, sizeof(*cmd));
    cmd->initiator        = (cec_logical_address)initiator;
    cmd->destination      = (cec_logical_address)destination;
    cmd->opcode           = (cec_opcode)opcode;
    cmd->opcode_set       = (int8_t)(opcode_set ? 1 : 0);
    cmd->transmit_timeout = 1000; /* CEC_DEFAULT_TRANSMIT_TIMEOUT */
}

/* Appends up to n bytes; caller has already bounds-checked n <= 64. */
void capi_command_push_params(cec_command *cmd, const uint8_t *data, int n) {
    if (!cmd || !data || n <= 0)
        return;
    if (n > CEC_MAX_DATA_PACKET_SIZE - cmd->parameters.size)
        n = CEC_MAX_DATA_PACKET_SIZE - cmd->parameters.size;
    for (int i = 0; i < n; i++)
        cmd->parameters.data[cmd->parameters.size++] = data[i];
}

/* ------------------------------------------------------------------ */
/* Flat wrappers that hide by-value / array struct returns.            */
/* ------------------------------------------------------------------ */

/* Fills paths/comms (each bufsize slots of slot bytes) and returns the
 * adapter count, or a negative value on failure. */
int8_t capi_find_adapters(void *handle, uint8_t bufsize, char *paths, char *comms,
                          size_t slot) {
    cec_adapter adapters[16];
    if (bufsize > 16)
        bufsize = 16;
    int8_t count = libcec_find_adapters((libcec_connection_t)handle, adapters, bufsize, NULL);
    if (count < 0)
        return count;
    for (int8_t i = 0; i < count && i < (int8_t)bufsize; i++) {
        strncpy(paths + (size_t)i * slot, adapters[i].path, slot - 1);
        paths[(size_t)i * slot + slot - 1] = '\0';
        strncpy(comms + (size_t)i * slot, adapters[i].comm, slot - 1);
        comms[(size_t)i * slot + slot - 1] = '\0';
    }
    return count;
}

uint16_t capi_get_active_devices_mask(void *handle) {
    cec_logical_addresses addrs = libcec_get_active_devices((libcec_connection_t)handle);
    uint16_t mask = 0;
    for (int i = 0; i < 16; i++)
        if (addrs.addresses[i])
            mask |= (uint16_t)(1u << i);
    return mask;
}

/* Returns the bitmask of this adapter's logical addresses; writes the
 * primary address through *primary_out. */
uint16_t capi_get_logical_addresses_mask(void *handle, int *primary_out) {
    cec_logical_addresses addrs = libcec_get_logical_addresses((libcec_connection_t)handle);
    if (primary_out)
        *primary_out = (int)addrs.primary;
    uint16_t mask = 0;
    for (int i = 0; i < 16; i++)
        if (addrs.addresses[i])
            mask |= (uint16_t)(1u << i);
    return mask;
}

/* Copies the OSD name (up to 14 chars + NUL) reported by the device into
 * out. Returns the libcec status code. */
int capi_get_device_osd_name(void *handle, int address, char *out, size_t out_size) {
    cec_osd_name name;
    memset(&name, 0, sizeof(name));
    int rc = libcec_get_device_osd_name((libcec_connection_t)handle,
                                        (cec_logical_address)address, name);
    if (out && out_size > 0) {
        strncpy(out, name, out_size - 1);
        out[out_size - 1] = '\0';
    }
    return rc;
}

/* Same pattern for the 3-char ISO 639-2 menu language. */
int capi_get_device_menu_language(void *handle, int address, char *out, size_t out_size) {
    cec_menu_language lang;
    memset(&lang, 0, sizeof(lang));
    int rc = libcec_get_device_menu_language((libcec_connection_t)handle,
                                             (cec_logical_address)address, lang);
    if (out && out_size > 0) {
        strncpy(out, lang, out_size - 1);
        out[out_size - 1] = '\0';
    }
    return rc;
}

/* ------------------------------------------------------------------ */
/* Read-back getters for configuration snapshots (config-changed event, */
/* get_current_configuration).                                          */
/* ------------------------------------------------------------------ */

const char *capi_config_device_name(const void *cfg) {
    return cfg ? ((const libcec_configuration *)cfg)->strDeviceName : "";
}

int capi_config_device_type(const void *cfg) {
    return cfg ? (int)((const libcec_configuration *)cfg)->deviceTypes.types[0] : 0;
}

uint16_t capi_config_physical_address(const void *cfg) {
    return cfg ? ((const libcec_configuration *)cfg)->iPhysicalAddress : 0;
}

int capi_config_base_device(const void *cfg) {
    return cfg ? (int)((const libcec_configuration *)cfg)->baseDevice : 0;
}

uint8_t capi_config_hdmi_port(const void *cfg) {
    return cfg ? ((const libcec_configuration *)cfg)->iHDMIPort : 0;
}

uint32_t capi_config_client_version(const void *cfg) {
    return cfg ? ((const libcec_configuration *)cfg)->clientVersion : 0;
}

uint32_t capi_config_server_version(const void *cfg) {
    return cfg ? ((const libcec_configuration *)cfg)->serverVersion : 0;
}

/* ------------------------------------------------------------------ */
/* Flat read-back of the running configuration.                        */
/* ------------------------------------------------------------------ */

/* Our own plain-C snapshot struct (NOT a libcec layout), so Rust can
 * receive the result of libcec_get_current_configuration without ever
 * naming libcec_configuration. Mirrored by ffi::capi_config_out. */
typedef struct capi_config_out {
    char device_name[LIBCEC_OSD_NAME_SIZE];
    int device_type;
    uint16_t physical_address;
    int base_device;
    uint8_t hdmi_port;
    uint32_t client_version;
    uint32_t server_version;
} capi_config_out;

int capi_get_current_configuration(void *handle, capi_config_out *out) {
    if (!out)
        return 0;
    libcec_configuration cfg;
    memset(&cfg, 0, sizeof(cfg));
    if (libcec_get_current_configuration((libcec_connection_t)handle, &cfg) == 0)
        return 0;
    memset(out, 0, sizeof(*out));
    strncpy(out->device_name, cfg.strDeviceName, LIBCEC_OSD_NAME_SIZE - 1);
    out->device_type      = (int)cfg.deviceTypes.types[0];
    out->physical_address = cfg.iPhysicalAddress;
    out->base_device      = (int)cfg.baseDevice;
    out->hdmi_port        = cfg.iHDMIPort;
    out->client_version   = cfg.clientVersion;
    out->server_version   = cfg.serverVersion;
    return 1;
}

void capi_command_set_transmit_timeout(cec_command *cmd, int32_t ms) {
    if (cmd)
        cmd->transmit_timeout = ms;
}

/* Flat getters for cec_command scalar fields (used by the command bridge
 * so Rust never touches the struct layout). */
uint8_t capi_command_initiator(const cec_command *cmd) {
    return cmd ? (uint8_t)cmd->initiator : 0xFF;
}

uint8_t capi_command_destination(const cec_command *cmd) {
    return cmd ? (uint8_t)cmd->destination : 0xFF;
}

int capi_command_ack(const cec_command *cmd) {
    return cmd ? (cmd->ack != 0) : 0;
}

int capi_command_eom(const cec_command *cmd) {
    return cmd ? (cmd->eom != 0) : 0;
}

uint8_t capi_command_opcode(const cec_command *cmd) {
    return cmd ? (uint8_t)cmd->opcode : 0;
}

uint8_t capi_command_opcode_set(const cec_command *cmd) {
    return cmd ? (uint8_t)(cmd->opcode_set != 0) : 0;
}
