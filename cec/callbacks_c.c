// C callback implementations that bridge libCEC callbacks into Go.
// This file is compiled as part of the cec package via cgo.

#include <libcec/cecc.h>
#include <stdint.h>

// Forward declarations of Go bridge functions (implemented in callbacks.go).
extern void goLogMessageCallbackBridge(void* handle, int level, int64_t timestamp, char* message);
extern void goKeyPressCallbackBridge(void* handle, int keycode, unsigned int duration);
extern void goCommandCallbackBridge(void* handle, void* command);
extern void goConfigurationChangedCallbackBridge(void* handle, void* config);
extern void goAlertCallbackBridge(void* handle, int alert, void* param);
extern int  goMenuStateChangedCallbackBridge(void* handle, int state);
extern void goSourceActivatedCallbackBridge(void* handle, int address, int activated);

void goLogMessageCallback(void* handle, const cec_log_message* message) {
    if (message && message->message) {
        goLogMessageCallbackBridge(handle, message->level, message->time, (char*)message->message);
    }
}

void goKeyPressCallback(void* handle, const cec_keypress* key) {
    if (key) {
        goKeyPressCallbackBridge(handle, key->keycode, key->duration);
    }
}

void goCommandCallback(void* handle, const cec_command* command) {
    if (command) {
        goCommandCallbackBridge(handle, (void*)command);
    }
}

void goConfigurationChangedCallback(void* handle, const libcec_configuration* config) {
    if (config) {
        goConfigurationChangedCallbackBridge(handle, (void*)config);
    }
}

void goAlertCallback(void* handle, const libcec_alert alert, const libcec_parameter param) {
    goAlertCallbackBridge(handle, alert, (void*)&param);
}

int goMenuStateChangedCallback(void* handle, const cec_menu_state state) {
    return goMenuStateChangedCallbackBridge(handle, state);
}

void goSourceActivatedCallback(void* handle, const cec_logical_address address, const uint8_t activated) {
    goSourceActivatedCallbackBridge(handle, address, activated);
}

