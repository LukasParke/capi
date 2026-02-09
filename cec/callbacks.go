package cec

/*
#include <libcec/cecc.h>
*/
import "C"

import (
	"unsafe"
)

//export goLogMessageCallbackBridge
func goLogMessageCallbackBridge(handle unsafe.Pointer, level C.int, time C.int64_t, message *C.char) {
	connectionsMu.RLock()
	conn, ok := connections[C.libcec_connection_t(handle)]
	connectionsMu.RUnlock()

	if !ok {
		return
	}

	conn.mu.Lock()
	callbacks := conn.callbacks
	conn.mu.Unlock()

	if callbacks != nil {
		msg := C.GoString(message)
		callbacks.OnLogMessage(LogLevel(level), int64(time), msg)
	}
}

//export goKeyPressCallbackBridge
func goKeyPressCallbackBridge(handle unsafe.Pointer, keycode C.int, duration C.uint) {
	connectionsMu.RLock()
	conn, ok := connections[C.libcec_connection_t(handle)]
	connectionsMu.RUnlock()

	if !ok {
		return
	}

	conn.mu.Lock()
	callbacks := conn.callbacks
	conn.mu.Unlock()

	if callbacks != nil {
		callbacks.OnKeyPress(Keycode(keycode), uint32(duration))
	}
}

//export goCommandCallbackBridge
func goCommandCallbackBridge(handle unsafe.Pointer, commandPtr unsafe.Pointer) {
	connectionsMu.RLock()
	conn, ok := connections[C.libcec_connection_t(handle)]
	connectionsMu.RUnlock()

	if !ok {
		return
	}

	conn.mu.Lock()
	callbacks := conn.callbacks
	conn.mu.Unlock()

	if callbacks != nil {
		cCmd := (*C.cec_command)(commandPtr)

		params := make([]uint8, cCmd.parameters.size)
		for i := 0; i < int(cCmd.parameters.size); i++ {
			params[i] = uint8(cCmd.parameters.data[i])
		}

		cmd := &Command{
			Initiator:    LogicalAddress(cCmd.initiator),
			Destination:  LogicalAddress(cCmd.destination),
			Ack:          cCmd.ack != 0,
			Eom:          cCmd.eom != 0,
			Opcode:       Opcode(cCmd.opcode),
			OpcodeSet:    cCmd.opcode_set != 0,
			Parameters:   params,
			TransmitTime: int64(cCmd.transmit_timeout),
		}

		callbacks.OnCommand(cmd)
	}
}

//export goConfigurationChangedCallbackBridge
func goConfigurationChangedCallbackBridge(handle unsafe.Pointer, configPtr unsafe.Pointer) {
	connectionsMu.RLock()
	conn, ok := connections[C.libcec_connection_t(handle)]
	connectionsMu.RUnlock()

	if !ok {
		return
	}

	conn.mu.Lock()
	callbacks := conn.callbacks
	conn.mu.Unlock()

	if callbacks != nil {
		cConfig := (*C.libcec_configuration)(configPtr)

		config := &Configuration{
			DeviceName:      C.GoString(&cConfig.strDeviceName[0]),
			DeviceType:      DeviceType(cConfig.deviceTypes.types[0]),
			PhysicalAddress: uint16(cConfig.iPhysicalAddress),
			BaseDevice:      LogicalAddress(cConfig.baseDevice),
			HDMIPort:        uint8(cConfig.iHDMIPort),
			ClientVersion:   uint32(cConfig.clientVersion),
			ServerVersion:   uint32(cConfig.serverVersion),
		}

		callbacks.OnConfigurationChanged(config)
	}
}

//export goAlertCallbackBridge
func goAlertCallbackBridge(handle unsafe.Pointer, alert C.int, paramPtr unsafe.Pointer) {
	connectionsMu.RLock()
	conn, ok := connections[C.libcec_connection_t(handle)]
	connectionsMu.RUnlock()

	if !ok {
		return
	}

	conn.mu.Lock()
	callbacks := conn.callbacks
	conn.mu.Unlock()

	if callbacks != nil {
		// Simple parameter handling - can be extended
		param := Parameter{
			Type: int(alert),
		}
		callbacks.OnAlert(Alert(alert), param)
	}
}

//export goMenuStateChangedCallbackBridge
func goMenuStateChangedCallbackBridge(handle unsafe.Pointer, state C.int) C.int {
	connectionsMu.RLock()
	conn, ok := connections[C.libcec_connection_t(handle)]
	connectionsMu.RUnlock()

	if !ok {
		return 0
	}

	conn.mu.Lock()
	callbacks := conn.callbacks
	conn.mu.Unlock()

	if callbacks != nil {
		if callbacks.OnMenuStateChanged(MenuState(state)) {
			return 1
		}
	}
	return 0
}

//export goSourceActivatedCallbackBridge
func goSourceActivatedCallbackBridge(handle unsafe.Pointer, address C.int, activated C.int) {
	connectionsMu.RLock()
	conn, ok := connections[C.libcec_connection_t(handle)]
	connectionsMu.RUnlock()

	if !ok {
		return
	}

	conn.mu.Lock()
	callbacks := conn.callbacks
	conn.mu.Unlock()

	if callbacks != nil {
		callbacks.OnSourceActivated(LogicalAddress(address), activated != 0)
	}
}
