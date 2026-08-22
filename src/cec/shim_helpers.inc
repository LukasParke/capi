#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

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



typedef struct capi_config_out {
    char device_name[LIBCEC_OSD_NAME_SIZE];
    int device_type;
    uint16_t physical_address;
    int base_device;
    uint8_t hdmi_port;
    uint32_t client_version;
    uint32_t server_version;
} capi_config_out;

void capi_apply_address_list(cec_logical_addresses* dest, const uint8_t* addrs, int n);
void capi_set_passive_defaults(libcec_configuration* cfg);

void capi_set_passive_defaults(libcec_configuration *cfg) {
    if (!cfg)
        return;
    cfg->bActivateSource = 0;
    cfg->bMonitorOnly    = 0;
    capi_apply_address_list(&cfg->wakeDevices, NULL, 0);
    capi_apply_address_list(&cfg->powerOffDevices, NULL, 0);
}

/* Shared stateless helpers included by both real and mock backends. */
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

uint8_t capi_command_initiator(const cec_command *cmd) {
    return cmd ? (uint8_t)cmd->initiator : 0xFF;
}

uint8_t capi_command_destination(const cec_command *cmd) {
    return cmd ? (uint8_t)cmd->destination : 0xFF;
}

uint8_t capi_command_opcode(const cec_command *cmd) {
    return cmd ? (uint8_t)cmd->opcode : 0;
}

uint8_t capi_command_opcode_set(const cec_command *cmd) {
    return cmd ? (uint8_t)(cmd->opcode_set != 0) : 0;
}

int capi_command_eom(const cec_command *cmd) {
    return cmd ? (cmd->eom != 0) : 0;
}

int capi_command_ack(const cec_command *cmd) {
    return cmd ? (cmd->ack != 0) : 0;
}

uint8_t capi_command_param_size(const cec_command *cmd) {
    return cmd->parameters.size;
}

uint8_t capi_command_param_byte(const cec_command *cmd, int i) {
    return cmd->parameters.data[i];
}

void capi_command_push_params(cec_command *cmd, const uint8_t *data, int n) {
    if (!cmd || !data || n <= 0)
        return;
    if (n > CEC_MAX_DATA_PACKET_SIZE - cmd->parameters.size)
        n = CEC_MAX_DATA_PACKET_SIZE - cmd->parameters.size;
    for (int i = 0; i < n; i++)
        cmd->parameters.data[cmd->parameters.size++] = data[i];
}

void capi_command_set_transmit_timeout(cec_command *cmd, int32_t ms) {
    if (cmd)
        cmd->transmit_timeout = ms;
}

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

uint32_t capi_client_version(void) {
    return (uint32_t)LIBCEC_VERSION_CURRENT;
}

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

uint16_t capi_get_active_devices_mask(void *handle) {
    cec_logical_addresses addrs = libcec_get_active_devices((libcec_connection_t)handle);
    uint16_t mask = 0;
    for (int i = 0; i < 16; i++)
        if (addrs.addresses[i])
            mask |= (uint16_t)(1u << i);
    return mask;
}

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

void capi_set_bridges(const capi_bridges *bridges) {
    if (bridges)
        g_bridges = *bridges;
}



/* Callback thunks + installation (shared with mock). */

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

ICECCallbacks *capi_callback_table(void) {
    static ICECCallbacks tbl;
    static int initialized = 0;
    // Atomic publish: two sessions initialising concurrently (supervisor
    // reconnect racing a probe, or parallel test threads) must never observe
    // a half-populated table.
    int expected = 0;
    if (__atomic_compare_exchange_n(&initialized, &expected, 1, false,
                                    __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST)) {
        memset(&tbl, 0, sizeof(tbl));
        tbl.logMessage           = cec_c_log;
        tbl.keyPress             = cec_c_key;
        tbl.commandReceived      = cec_c_command;
        tbl.configurationChanged = cec_c_config;
        tbl.alert                = cec_c_alert;
        tbl.menuStateChanged     = cec_c_menu;
        tbl.sourceActivated      = cec_c_source;
        __atomic_store_n(&initialized, 2, __ATOMIC_SEQ_CST);
    } else {
        while (__atomic_load_n(&initialized, __ATOMIC_SEQ_CST) != 2) {
            /* spin: winner is mid-fill; table writes are plain stores that
             * complete before the release store above */
        }
    }
    return &tbl;
}

void capi_install_callbacks(libcec_configuration *cfg, uintptr_t id) {
    if (!cfg)
        return;
    cfg->callbacks     = capi_callback_table();
    cfg->callbackParam = (void *)(uintptr_t)id;
}
