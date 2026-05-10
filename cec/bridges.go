package cec

/*
#include <libcec/cecc.h>
#include <stdint.h>

extern ICECCallbacks* cec_callback_table(void);
extern uint8_t cec_command_param_byte(const cec_command* cmd, int i);
extern uint8_t cec_command_param_size(const cec_command* cmd);
*/
import "C"

import (
	"runtime/cgo"
	"time"
)

// connFromHandle resolves a cgo.Handle uintptr (carried as cbParam) back to
// the owning *Connection. Returns nil if the handle has been deleted (which
// happens during Close, when libcec may still flush a final callback or two).
func connFromHandle(h C.uintptr_t) *Connection {
	if h == 0 {
		return nil
	}
	defer func() { _ = recover() }()
	v := cgo.Handle(uintptr(h)).Value()
	c, _ := v.(*Connection)
	return c
}

//export cec_bridge_log
func cec_bridge_log(handle C.uintptr_t, level C.int, ts C.int64_t, message *C.char) {
	c := connFromHandle(handle)
	if c == nil || c.closed.Load() {
		return
	}
	c.dispatch(Event{
		Kind:      EventLog,
		Timestamp: time.Now(),
		Log: &LogPayload{
			Level:   LogLevel(level),
			Time:    int64(ts),
			Message: C.GoString(message),
		},
	})
}

//export cec_bridge_key
func cec_bridge_key(handle C.uintptr_t, keycode C.int, duration C.uint) {
	c := connFromHandle(handle)
	if c == nil || c.closed.Load() {
		return
	}
	c.dispatch(Event{
		Kind:      EventKeyPress,
		Timestamp: time.Now(),
		Key: &KeyPayload{
			Key:      Keycode(keycode),
			Duration: uint32(duration),
		},
	})
}

//export cec_bridge_command
func cec_bridge_command(handle C.uintptr_t, cmd *C.cec_command) {
	c := connFromHandle(handle)
	if c == nil || c.closed.Load() || cmd == nil {
		return
	}
	size := int(C.cec_command_param_size(cmd))
	params := make([]uint8, size)
	for i := 0; i < size; i++ {
		params[i] = uint8(C.cec_command_param_byte(cmd, C.int(i)))
	}
	c.dispatch(Event{
		Kind:      EventCommand,
		Timestamp: time.Now(),
		Command: &Command{
			Initiator:    LogicalAddress(cmd.initiator),
			Destination:  LogicalAddress(cmd.destination),
			Ack:          cmd.ack != 0,
			Eom:          cmd.eom != 0,
			Opcode:       Opcode(cmd.opcode),
			OpcodeSet:    cmd.opcode_set != 0,
			Parameters:   params,
			TransmitTime: int64(cmd.transmit_timeout),
		},
	})
}

//export cec_bridge_config
func cec_bridge_config(handle C.uintptr_t, cfg *C.libcec_configuration) {
	c := connFromHandle(handle)
	if c == nil || c.closed.Load() || cfg == nil {
		return
	}
	out := &Configuration{
		DeviceName:      C.GoString(&cfg.strDeviceName[0]),
		DeviceType:      DeviceType(cfg.deviceTypes.types[0]),
		PhysicalAddress: uint16(cfg.iPhysicalAddress),
		BaseDevice:      LogicalAddress(cfg.baseDevice),
		HDMIPort:        uint8(cfg.iHDMIPort),
		ClientVersion:   uint32(cfg.clientVersion),
		ServerVersion:   uint32(cfg.serverVersion),
	}
	c.dispatch(Event{
		Kind:      EventConfigChanged,
		Timestamp: time.Now(),
		Config:    out,
	})
}

//export cec_bridge_alert
func cec_bridge_alert(handle C.uintptr_t, alert C.int, paramType C.int, paramValue C.int64_t) {
	c := connFromHandle(handle)
	if c == nil || c.closed.Load() {
		return
	}
	c.dispatch(Event{
		Kind:      EventAlert,
		Timestamp: time.Now(),
		Alert: &AlertPayload{
			Alert: Alert(alert),
			Param: Parameter{Type: int(paramType), Value: int64(paramValue)},
		},
	})
}

//export cec_bridge_menu
func cec_bridge_menu(handle C.uintptr_t, state C.int) C.int {
	c := connFromHandle(handle)
	if c == nil || c.closed.Load() {
		return 1
	}
	fn := c.menuFn.Load()
	if fn != nil && *fn != nil {
		if (*fn)(MenuState(state)) {
			return 1
		}
		return 0
	}
	return 1
}

//export cec_bridge_source
func cec_bridge_source(handle C.uintptr_t, address C.int, activated C.int) {
	c := connFromHandle(handle)
	if c == nil || c.closed.Load() {
		return
	}
	c.dispatch(Event{
		Kind:      EventSourceActivated,
		Timestamp: time.Now(),
		Source: &SourcePayload{
			Address:   LogicalAddress(uint8(address)),
			Activated: activated != 0,
		},
	})
}

