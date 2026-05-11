package main

import (
	"fmt"
	"log"

	"github.com/LukasParke/capi/cec"
)

// runCECEventConsumer drains the cec.Connection event channel and translates
// each event into the appropriate side effects: log buffer, bus state update,
// frame ring, EventHub publish, topology hint, and reconnect signals.
//
// The goroutine returns when the channel is closed (which happens on
// conn.Close).
func runCECEventConsumer(conn *cec.Connection) {
	for ev := range conn.Events() {
		switch ev.Kind {
		case cec.EventLog:
			if ev.Log != nil {
				handleCECLog(ev.Log)
			}
		case cec.EventKeyPress:
			if ev.Key != nil {
				handleCECKeyPress(ev.Key)
			}
		case cec.EventCommand:
			if ev.Command != nil {
				handleCECCommand(ev.Command)
			}
		case cec.EventConfigChanged:
			if ev.Config != nil {
				handleCECConfigChanged(ev.Config)
			}
		case cec.EventAlert:
			if ev.Alert != nil {
				handleCECAlert(ev.Alert)
			}
		case cec.EventSourceActivated:
			if ev.Source != nil {
				handleCECSourceActivated(ev.Source)
			}
		}
	}
}

func handleCECLog(p *cec.LogPayload) {
	if logHandler != nil {
		logHandler.RecordCEC(p.Level, p.Time, p.Message)
	}
}

func handleCECKeyPress(p *cec.KeyPayload) {
	log.Printf("Key pressed: %d, duration: %d", p.Key, p.Duration)
	if eventHub != nil {
		eventHub.Publish(CECEvent{
			Type: "key_press",
			Data: map[string]interface{}{
				"keycode":  int(p.Key),
				"duration": p.Duration,
			},
		})
	}
}

func handleCECCommand(cmd *cec.Command) {
	log.Printf("Command received: %s -> %s, opcode: 0x%02X",
		cmd.Initiator.String(), cmd.Destination.String(), cmd.Opcode)
	// Note every initiator we've ever heard, even when the opcode itself
	// doesn't have a specific recordObserved branch. This is what lets
	// ghost devices (e.g. the receiver behind a non-bridging projector)
	// appear in /api/devices the moment they ever speak.
	if cmd.Initiator <= cec.LogicalAddressFreeUse {
		globalBusState.noteSeen(int(cmd.Initiator))
	}
	globalBusState.ApplyObservedFromCECCCommand(cmd)
	cfg := busConfigLocked()
	if ringCap := cfg.frameRingSize(); ringCap > 0 {
		globalBusState.appendFrameRing(cmd, ringCap)
	}
	if eventHub != nil {
		params := make([]string, len(cmd.Parameters))
		for i, b := range cmd.Parameters {
			params[i] = fmt.Sprintf("0x%02X", b)
		}
		data := map[string]interface{}{
			"initiator":    int(cmd.Initiator),
			"destination":  int(cmd.Destination),
			"opcode":       fmt.Sprintf("0x%02X", cmd.Opcode),
			"opcode_set":   cmd.OpcodeSet,
			"ack":          cmd.Ack,
			"eom":          cmd.Eom,
			"parameters":   params,
			"params_bytes": cmd.Parameters,
		}
		switch {
		case cmd.Opcode == cec.OpcodeReportPowerStatus && len(cmd.Parameters) >= 1:
			eventHub.Publish(CECEvent{
				Type: "power_change",
				Data: map[string]interface{}{
					"address": int(cmd.Initiator),
					"status":  powerStatusFromByte(cmd.Parameters[0]),
				},
			})
		case cmd.Opcode == cec.OpcodeStandby:
			eventHub.Publish(CECEvent{
				Type: "power_change",
				Data: map[string]interface{}{
					"address": int(cmd.Initiator),
					"status":  "standby",
				},
			})
		}
		eventHub.Publish(CECEvent{Type: "command", Data: data})
	}
	if tier := opcodeTopologyTier(cmd.Opcode); tier != topologyNone {
		signalBusTopologyHint(tier)
	}
}

func handleCECConfigChanged(c *cec.Configuration) {
	log.Printf("Configuration changed: %s", c.DeviceName)
	if eventHub != nil {
		eventHub.Publish(CECEvent{
			Type: "configuration_changed",
			Data: map[string]interface{}{
				"device_name": c.DeviceName,
			},
		})
	}
	signalBusTopologyHint(topologyHeavy)
}

func handleCECAlert(p *cec.AlertPayload) {
	log.Printf("Alert: %d param=%d", p.Alert, p.Param.Value)
	appLog("cec", "libCEC alert code=%d param=%d", int(p.Alert), int(p.Param.Value))
	if eventHub != nil {
		eventHub.Publish(CECEvent{
			Type: "alert",
			Data: map[string]interface{}{
				"alert": int(p.Alert),
				"param": p.Param.Value,
			},
		})
	}
	switch p.Alert {
	case cec.AlertConnectionLost, cec.AlertPermissionError, cec.AlertPortBusy:
		signalCECReconnect()
	}
}

func handleCECSourceActivated(p *cec.SourcePayload) {
	log.Printf("Source activated: %s, activated: %v", p.Address.String(), p.Activated)
	if eventHub != nil {
		eventHub.Publish(CECEvent{
			Type: "source_activated",
			Data: map[string]interface{}{
				"address":   int(p.Address),
				"activated": p.Activated,
			},
		})
	}
}

// powerStatusFromByte maps a raw CEC power status byte to a human string.
func powerStatusFromByte(b uint8) string {
	switch b {
	case 0x00:
		return "on"
	case 0x01:
		return "standby"
	case 0x02:
		return "transitioning_to_on"
	case 0x03:
		return "transitioning_to_standby"
	default:
		return "unknown"
	}
}
