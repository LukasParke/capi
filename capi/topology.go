package main

import (
	"log"
	"time"

	"github.com/LukasParke/capi/cec"
)

// topologyTier classifies how aggressively the bus steward should refresh.
type topologyTier int

const (
	topologyNone topologyTier = iota
	topologyLight
	topologyHeavy
)

// opcodeTopologyTier maps a bus opcode to a steward refresh tier.
// Heavy: routing / identity changes that benefit from RescanDevices.
// Light: status-only frames; refresh without full rescan.
func opcodeTopologyTier(op cec.Opcode) topologyTier {
	switch op {
	case cec.OpcodeReportPhysicalAddress,
		cec.OpcodeDeviceVendorID,
		cec.OpcodeSetOSDName,
		cec.OpcodeActiveSource,
		cec.OpcodeRoutingChange,
		cec.OpcodeRoutingInformation,
		cec.OpcodeSetStreamPath,
		cec.OpcodeInactiveSource,
		cec.OpcodeRequestActiveSource:
		return topologyHeavy
	case cec.OpcodeReportPowerStatus, cec.OpcodeReportAudioStatus:
		return topologyLight
	default:
		return topologyNone
	}
}

// opcodeRequestsTopologyRescan is kept for tests: true when any refresh is useful.
func opcodeRequestsTopologyRescan(op cec.Opcode) bool {
	return opcodeTopologyTier(op) != topologyNone
}

var topologyHintCh = make(chan topologyTier, 16)

func signalBusTopologyHint(tier topologyTier) {
	if tier == topologyNone {
		return
	}
	select {
	case topologyHintCh <- tier:
	default:
		// Drop if saturated; next opcode will likely re-hint.
	}
}

// runBusTopologyWorkerLoop coalesces bus traffic hints and enqueues steward work
// (no direct CEC calls — serialized in the steward).
func runBusTopologyWorkerLoop() {
	const debounce = 500 * time.Millisecond
	for {
		first := <-topologyHintCh
		pending := first
		time.Sleep(debounce)
		for len(topologyHintCh) > 0 {
			t := <-topologyHintCh
			if t == topologyHeavy {
				pending = topologyHeavy
			} else if t == topologyLight && pending != topologyHeavy {
				pending = topologyLight
			}
		}

		switch pending {
		case topologyHeavy:
			signalStewardFull()
		case topologyLight:
			signalStewardLight()
		default:
			log.Printf("topology: unexpected pending tier %d", pending)
		}
	}
}
