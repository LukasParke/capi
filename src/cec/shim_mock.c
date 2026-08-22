/* Mock libcec backend for integration tests (feature = "mock-cec").
 *
 * Implements every symbol the Rust FFI layer declares, backed by a virtual
 * adapter: TV (LA 0), playback device "MOCKBOX" (LA 4, vendor 0x809819),
 * audio system (LA 5). Transmits always ack and are recorded for assertions.
 * mock_emit_command() drives the REAL callback -> dispatch chain so bridge
 * success paths are covered without hardware.
 *
 * Never linked into release builds.
 */
#define _GNU_SOURCE
#include <libcec/cecc.h>
#pragma GCC diagnostic ignored "-Wunused-parameter"
#pragma GCC diagnostic ignored "-Wmisleading-indentation"
#pragma GCC diagnostic ignored "-Warray-parameter"
#pragma GCC diagnostic ignored "-Warray-parameter"
#include "shim_helpers.inc"
#include <stdint.h>
#include <string.h>
#include <time.h>
#include <stdio.h>

static ICECCallbacks mock_cb_table;
static void *mock_cb_param = NULL;
static int mock_session_open = 0;
static char mock_lib_info[128];

/* Last transmit record (observable via mock_last_transmit). */
static uint8_t tx_initiator, tx_dest, tx_opcode;
static uint8_t tx_params[64];
static int tx_params_len, tx_count;

static int last_was_reply;
int mock_last_was_reply(void) { return last_was_reply; }

static int fail_remaining;

int mock_should_fail(void) {
    if (fail_remaining > 0) {
        fail_remaining--;
        return 1;
    }
    return 0;
}

void mock_set_fail_next(int n) { fail_remaining = n; }

void mock_reset(void) {
    tx_initiator = tx_dest = tx_opcode = 0;
    tx_params_len = 0;
    tx_count = 0;
    last_was_reply = 0;
    fail_remaining = 0;
    mock_session_open = 0;
}

int mock_last_transmit(uint8_t *initiator, uint8_t *dest, uint8_t *opcode,
                       uint8_t *params_out, int cap) {
    *initiator = tx_initiator;
    *dest = tx_dest;
    *opcode = tx_opcode;
    int n = tx_params_len < cap ? tx_params_len : cap;
    memcpy(params_out, tx_params, n);
    return n; /* number of bytes written to params_out */
}

int mock_session_is_open(void) { return mock_session_open; }

/* Inject a bus frame through the production callback chain. */
void mock_emit_command_on(uintptr_t id, uint8_t initiator, uint8_t dest,
                          uint8_t opcode, const uint8_t *params, int len) {
    if (!g_bridges.command) return;
    cec_command cmd;
    memset(&cmd, 0, sizeof(cmd));
    cmd.initiator = (cec_logical_address)initiator;
    cmd.destination = (cec_logical_address)dest;
    cmd.opcode = (cec_opcode)opcode;
    cmd.opcode_set = 1;
    cmd.eom = 1;
    cmd.parameters.size = (uint8_t)(len > 64 ? 64 : len);
    for (int i = 0; i < cmd.parameters.size; i++)
        cmd.parameters.data[i] = params[i];
    /* Flatten params through the same helper the real path uses so the
       bridge sees an identical shape. */
    g_bridges.command(id, &cmd);
}

void mock_emit_keypress_on(uintptr_t id, uint8_t key, uint32_t duration) {
    if (!g_bridges.key) return;
    g_bridges.key(id, (int)key, duration);
}

/* ---- lifecycle ----------------------------------------------------------- */

static const char MOCK_INFO[] =
    "mock libcec (feature mock-cec), features: MOCK";

void *libcec_initialise(libcec_configuration *cfg) {
    if (cfg && cfg->callbacks) {
        mock_cb_table = *cfg->callbacks;
        mock_cb_param = cfg->callbackParam;
    }
    snprintf(mock_lib_info, sizeof(mock_lib_info), "%s", MOCK_INFO);
    /* Non-null sentinel handle. */
    return (void *)&mock_session_open;
}

void libcec_destroy(void *handle) { (void)handle; mock_session_open = 0; }

int libcec_open(void *handle, const char *port, uint32_t timeout) {
    if (mock_should_fail()) return 0;
    (void)handle; (void)port; (void)timeout;
    mock_session_open = 1;
    return 1;
}

void libcec_close(void *handle) { (void)handle; mock_session_open = 0; }

int8_t libcec_find_adapters(void *h, cec_adapter *list, uint8_t cap, const char *path) {
    if (mock_should_fail()) return 0;
    (void)h; (void)path;
    if (cap < 1) return 0;
    memset(&list[0], 0, sizeof(list[0]));
    strcpy(list[0].path, "/dev/mock0");
    strcpy(list[0].comm, "/dev/mock0");
    return 1;
}

const char *libcec_get_lib_info(void *h) { (void)h; return mock_lib_info; }

/* ---- power / source / standby -------------------------------------------- */

int libcec_power_on_devices(void *h, cec_logical_address a) {
    if (mock_should_fail()) return 0; (void)h; return 1; }
int libcec_standby_devices(void *h, cec_logical_address a) {
    if (mock_should_fail()) return 0; (void)h; return 1; }
int libcec_set_active_source(void *h, cec_device_type t) {
    if (mock_should_fail()) return 0; (void)h; return 1; }
int libcec_set_inactive_view(void *h) {
    if (mock_should_fail()) return 0; (void)h; return 1; }
int libcec_switch_monitoring(void *h, int enable) {
    if (mock_should_fail()) return 0; (void)h; return 1; }
void libcec_rescan_devices(void *h) { (void)h; }

cec_power_status libcec_get_device_power_status(void *h, cec_logical_address a) {
    if (mock_should_fail()) return 0x99;
    (void)h; (void)a;
    return 0x00; /* on */
}

uint16_t libcec_get_device_physical_address(void *h, cec_logical_address a) {
    if (mock_should_fail()) return 0xFFFF;
    (void)h;
    return a == CECDEVICE_BROADCAST ? 0xFFFF : 0x2000; /* TV input 2 */
}

uint32_t libcec_get_device_vendor_id(void *h, cec_logical_address a) {
    if (mock_should_fail()) return 0;
    (void)h; (void)a;
    return 0x809819; /* Samsung-ish */
}

int libcec_get_device_osd_name(void *h, cec_logical_address a, char *out14) {
    if (mock_should_fail()) return 0;
    (void)h; (void)a;
    memcpy(out14, "MOCKBOX", 8);
    return 1;
}

int libcec_get_device_menu_language(void *h, cec_logical_address a, char *out4) {
    if (mock_should_fail()) return 0;
    (void)h; (void)a;
    memcpy(out4, "eng", 4);
    return 1;
}

cec_version libcec_get_device_cec_version(void *h, cec_logical_address a) {
    if (mock_should_fail()) return CEC_VERSION_UNKNOWN;
    (void)h; (void)a;
    return CEC_VERSION_1_4;
}

cec_logical_address libcec_get_active_source(void *h) {
    if (mock_should_fail()) return CECDEVICE_UNKNOWN;
    (void)h;
    return CECDEVICE_PLAYBACKDEVICE1;
}

int libcec_is_active_source(void *h, cec_logical_address a) {
    (void)h;
    return a == CECDEVICE_PLAYBACKDEVICE1 ? 1 : 0;
}

cec_logical_addresses libcec_get_active_devices(void *h) {
    if (mock_should_fail()) { cec_logical_addresses z; memset(&z,0,sizeof(z)); z.primary = CECDEVICE_UNKNOWN; return z; }
    (void)h;
    cec_logical_addresses out;
    memset(&out, 0, sizeof(out));
    out.addresses[0] = 1; /* TV */
    out.addresses[4] = 1; /* playback */
    out.addresses[5] = 1; /* audio */
    out.primary = CECDEVICE_PLAYBACKDEVICE1;
    return out;
}

int libcec_is_active_device(void *h, cec_logical_address a) {
    (void)h;
    return (a == CECDEVICE_TV || a == CECDEVICE_PLAYBACKDEVICE1 ||
            a == CECDEVICE_AUDIOSYSTEM)
               ? 1
               : 0;
}

int libcec_poll_device(void *h, cec_logical_address a) {
    if (mock_should_fail()) return 0;
    (void)h;
    return (a == CECDEVICE_TV || a == CECDEVICE_PLAYBACKDEVICE1 ||
            a == CECDEVICE_AUDIOSYSTEM)
               ? 1
               : 0;
}

/* ---- transmit / keys ------------------------------------------------------ */

int libcec_transmit(void *h, const cec_command *cmd) {
    if (mock_should_fail()) return 0;
    (void)h;
    tx_initiator = (uint8_t)cmd->initiator;
    tx_dest = (uint8_t)cmd->destination;
    tx_opcode = (uint8_t)cmd->opcode;
    tx_params_len = cmd->parameters.size > 64 ? 64 : cmd->parameters.size;
    memcpy(tx_params, cmd->parameters.data, tx_params_len);
    fprintf(stderr, "[mock_tx] dest=%02x op=%02x size=%d p0=%02x p1=%02x\n",
            tx_dest, tx_opcode, tx_params_len, tx_params[0], tx_params[1]);
    tx_count++;

    /* Auto-reply so strategy/probe classifiers see realistic traffic. */
    if (!g_bridges.command) return 1;
    cec_command reply;
    memset(&reply, 0, sizeof(reply));
    reply.initiator = cmd->destination;
    reply.destination = cmd->initiator;
    reply.opcode_set = 1;
    reply.eom = 1;
    uint8_t rp[16]; int rn = 0;
    switch ((uint8_t)cmd->opcode) {
        case 0x83: /* GivePhysicalAddress -> ReportPhysicalAddress 2.0.0.0 */
            reply.opcode = (cec_opcode)0x84; rp[0]=0x20; rp[1]=0x00; rn=2; break;
        case 0x85: /* RequestActiveSource -> ActiveSource 1.0.0.0? we are playback at 2.0.0.0 */
            reply.opcode = (cec_opcode)0x82; rp[0]=0x20; rp[1]=0x00; rn=2; break;
        case 0x8C: /* GiveDeviceVendorID */
            reply.opcode = (cec_opcode)0x87; rp[0]=0x80; rp[1]=0x98; rp[2]=0x19; rn=3; break;
        case 0x46: /* GiveOSDName -> SetOSDName "MOCKBOX" */
            reply.opcode = (cec_opcode)0x47; memcpy(rp, "MOCKBOX", 7); rn=7; break;
        case 0x9F: /* GetCECVersion -> CECVersion 1.4 */
            reply.opcode = (cec_opcode)0x9E; rp[0]=0x05; rn=1; break;
        case 0x8F: /* GiveDevicePowerStatus -> ReportPowerStatus on */
            reply.opcode = (cec_opcode)0x90; rp[0]=0x00; rn=1; break;
        case 0x71: /* GiveAudioStatus */
            reply.opcode = (cec_opcode)0x7A; rp[0]=37; rn=1; break;
        case 0x44: /* UserControlPressed: audio system answers volume keys */
            if ((uint8_t)cmd->parameters.size >= 1) {
                uint8_t k = cmd->parameters.data[0];
                if (k == 0x41 /*VolumeUp*/ || k == 0x42 /*VolumeDown*/ || k == 0x43 /*Mute*/) {
                    reply.opcode = (cec_opcode)0x7A; /* ReportAudioStatus */
                    rp[0] = 37;
                    rn = 1;
                    break;
                }
            }
            return 1; /* ack only */
        default:
            return 1; /* ack only */
    }
    reply.parameters.size = (uint8_t)rn;
    for (int i = 0; i < rn; i++) reply.parameters.data[i] = rp[i];
    if (g_bridges.command && mock_cb_param)
        g_bridges.command((uintptr_t)mock_cb_param, &reply);
    return 1;
}

int libcec_send_keypress(void *h, cec_logical_address d, cec_user_control_code k, int wait) {
    if (mock_should_fail()) return 0;
    (void)h; (void)wait;
    /* Audio system answers volume/mute presses with ReportAudioStatus. */
    if (d == CECDEVICE_AUDIOSYSTEM
        && (k == 0x41 || k == 0x42 || k == 0x43)
        && g_bridges.command && mock_cb_param) {
        cec_command reply;
        memset(&reply, 0, sizeof(reply));
        reply.initiator = d;
        reply.destination = d == CECDEVICE_AUDIOSYSTEM ? CECDEVICE_PLAYBACKDEVICE1 : CECDEVICE_TV;
        reply.opcode = (cec_opcode)0x7A;
        reply.opcode_set = 1;
        reply.eom = 1;
        reply.parameters.size = 1;
        reply.parameters.data[0] = 37;
        last_was_reply = 1;
        g_bridges.command((uintptr_t)mock_cb_param, &reply);
    }
    return 1;
}

int libcec_send_key_release(void *h, cec_logical_address d, int wait) {
    if (mock_should_fail()) return 0;
    (void)h; (void)d; (void)wait;
    return 1;
}

int libcec_set_osd_string(void *h, cec_logical_address d, cec_display_control dc, const char *msg) {
    if (mock_should_fail()) return 0;
    (void)h; (void)d; (void)dc; (void)msg;
    return 1;
}

/* ---- audio ---------------------------------------------------------------- */

uint8_t libcec_audio_get_status(void *h) {
    if (mock_should_fail()) return 0; (void)h; return 37; } /* vol 37, unmuted */
int libcec_volume_up(void *h, int rel) {
    if (mock_should_fail()) return 0; (void)h; (void)rel; return 1; }
int libcec_volume_down(void *h, int rel) {
    if (mock_should_fail()) return 0; (void)h; (void)rel; return 1; }
uint8_t libcec_audio_toggle_mute(void *h) {
    if (mock_should_fail()) return 0; (void)h; return 1; }
uint8_t libcec_audio_mute(void *h) {
    if (mock_should_fail()) return 0; (void)h; return 1; }
uint8_t libcec_audio_unmute(void *h) {
    if (mock_should_fail()) return 0; (void)h; return 1; }
int libcec_system_audio_mode(void *h, int enable) {
    if (mock_should_fail()) return 0; (void)h; (void)enable; return 1; }

/* ---- configuration --------------------------------------------------------- */

int libcec_set_configuration(void *h, const libcec_configuration *cfg) {
    if (mock_should_fail()) return 0;
    (void)h; (void)cfg;
    return 1;
}

int libcec_get_current_configuration(void *h, libcec_configuration *cfg) {
    if (mock_should_fail()) return 0;
    (void)h;
    memset(cfg, 0, sizeof(*cfg));
    snprintf(cfg->strDeviceName, sizeof(cfg->strDeviceName), "MOCK");
    cfg->deviceTypes.types[0] = CEC_DEVICE_TYPE_RECORDING_DEVICE;
    cfg->serverVersion = LIBCEC_VERSION_CURRENT;
    cfg->clientVersion = LIBCEC_VERSION_CURRENT;
    return 1;
}

/* ---- HDMI port switching ---------------------------------------------------- */

int libcec_set_hdmi_port(void *h, cec_logical_address base, uint8_t port) {
    if (mock_should_fail()) return 0;
    (void)h; (void)base; (void)port;
    return 1;
}

/* ---- backend-specific: initialise / callbacks-on-set --------------------- */

void *capi_initialise(const char *device_name, int device_type,
                      uint16_t physical_address, int base_device, uint8_t hdmi_port,
                      int monitor_only, int activate_source,
                      const uint8_t *wake, int wake_n,
                      const uint8_t *poweroff, int poweroff_n,
                      uintptr_t cb_param) {
    (void)device_name; (void)device_type; (void)physical_address; (void)base_device;
    (void)hdmi_port; (void)monitor_only; (void)activate_source;
    (void)wake; (void)wake_n; (void)poweroff; (void)poweroff_n;
    /* Capture callback plumbing exactly like real firmware would. */
    libcec_configuration *cfg = capi_build_config(
        device_name, device_type, physical_address, base_device, hdmi_port,
        capi_client_version(), monitor_only, activate_source,
        wake, wake_n, poweroff, poweroff_n);
    if (!cfg)
        return NULL;
    capi_install_callbacks(cfg, cb_param);
    mock_cb_param = (void *)cb_param;
    snprintf(mock_lib_info, sizeof(mock_lib_info), "%s", MOCK_INFO);
    capi_free_config(cfg);
    mock_session_open = 0;
    return (void *)&mock_session_open; /* non-null sentinel */
}

int capi_install_callbacks_on_set(void *handle, const char *device_name, int device_type,
                                  uint16_t physical_address, int base_device, uint8_t hdmi_port,
                                  int monitor_only, int activate_source,
                                  const uint8_t *wake, int wake_n,
                                  const uint8_t *poweroff, int poweroff_n,
                                  uintptr_t cb_param) {
    (void)handle; (void)device_name; (void)device_type; (void)physical_address;
    (void)base_device; (void)hdmi_port; (void)monitor_only; (void)activate_source;
    (void)wake; (void)wake_n; (void)poweroff; (void)poweroff_n;
    return 1;
}

/* Missing-piece stub: helpers.inc's logical-address mask wrapper calls this;
 * without it the REAL libcec.so resolves (sentinel handle -> SIGSEGV). */
cec_logical_addresses libcec_get_logical_addresses(void *h) {
    (void)h;
    cec_logical_addresses out;
    memset(&out, 0, sizeof(out));
    out.addresses[0] = 1; /* TV */
    out.addresses[4] = 1; /* playback */
    out.addresses[5] = 1; /* audio */
    out.primary = CECDEVICE_PLAYBACKDEVICE1;
    return out;
}

void mock_emit_alert(uintptr_t id, int alert, int ptype, int64_t pvalue) {
    if (!g_bridges.alert) return;
    g_bridges.alert(id, alert, ptype, pvalue);
}

void mock_emit_config_changed(void) {
    libcec_configuration cfg;
    memset(&cfg, 0, sizeof(cfg));
    snprintf(cfg.strDeviceName, sizeof(cfg.strDeviceName), "MOCK");
    cfg.clientVersion = LIBCEC_VERSION_CURRENT;
    cfg.serverVersion = LIBCEC_VERSION_CURRENT;
    if (!g_bridges.config_changed) return;
    g_bridges.config_changed((uintptr_t)mock_cb_param, &cfg);
}

void mock_emit_source_activated(uintptr_t id, uint8_t addr, int activated) {
    if (!g_bridges.source) return;
    g_bridges.source(id, addr, activated ? 1 : 0);
}

int mock_emit_menu_on(uintptr_t id, int state) {
    if (!g_bridges.menu) return 0;
    return g_bridges.menu(id, state);
}
