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
#include "shim_helpers.inc"
#include <stdint.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>

/* ------------------------------------------------------------------ */
/* Process-wide bridge table, filled once from Rust at startup.        */
/* ------------------------------------------------------------------ */








/* ------------------------------------------------------------------ */
/* Callback thunks (port of cec/bridges_c.c).                          */
/* ------------------------------------------------------------------ */















/* Process-wide ICECCallbacks table; identical for every connection. The
 * per-session identity rides in libcec_configuration.callbackParam. */




/* ------------------------------------------------------------------ */
/* Configuration helpers (port of cec_set_passive_defaults et al).      */
/* ------------------------------------------------------------------ */

/* Clears dest and populates it with up to 16 entries. An empty/NULL list
 * clears it, suppressing libcec's bus-disrupting defaults
 * (wakeDevices={TV}, powerOffDevices={BROADCAST}). */




/* Builds a heap-allocated libcec_configuration from flat arguments:
 * zeroed via libcec_clear_configuration, passive defaults applied, then
 * caller overrides. Returns NULL on allocation failure. Free with
 * capi_free_config. */




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





/* Initializes a command frame and sets an explicit transmit timeout.
 * In C compilation units cec_command's Clear() constructor never runs, so
 * transmit_timeout would stay 0 (the latent Go bug); we set it explicitly. */


/* Appends up to n bytes; caller has already bounds-checked n <= 64. */


/* ------------------------------------------------------------------ */
/* Flat wrappers that hide by-value / array struct returns.            */
/* ------------------------------------------------------------------ */

/* Fills paths/comms (each bufsize slots of slot bytes) and returns the
 * adapter count, or a negative value on failure. */




/* Returns the bitmask of this adapter's logical addresses; writes the
 * primary address through *primary_out. */


/* Copies the OSD name (up to 14 chars + NUL) reported by the device into
 * out. Returns the libcec status code. */


/* Same pattern for the 3-char ISO 639-2 menu language. */


/* ------------------------------------------------------------------ */
/* Read-back getters for configuration snapshots (config-changed event, */
/* get_current_configuration).                                          */
/* ------------------------------------------------------------------ */















/* ------------------------------------------------------------------ */
/* Flat read-back of the running configuration.                        */
/* ------------------------------------------------------------------ */

/* Our own plain-C snapshot struct (NOT a libcec layout), so Rust can
 * receive the result of libcec_get_current_configuration without ever
 * naming libcec_configuration. Mirrored by ffi::capi_config_out. */






/* Flat getters for cec_command scalar fields (used by the command bridge
 * so Rust never touches the struct layout). */











