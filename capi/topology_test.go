package main

import (
	"testing"

	"github.com/LukasParke/capi/cec"
)

func TestOpcodeTopologyTier(t *testing.T) {
	heavy := []cec.Opcode{
		cec.OpcodeReportPhysicalAddress,
		cec.OpcodeDeviceVendorID,
		cec.OpcodeSetOSDName,
		cec.OpcodeActiveSource,
		cec.OpcodeRoutingChange,
		cec.OpcodeRoutingInformation,
		cec.OpcodeSetStreamPath,
		cec.OpcodeInactiveSource,
		cec.OpcodeRequestActiveSource,
	}
	for _, op := range heavy {
		if opcodeTopologyTier(op) != topologyHeavy {
			t.Errorf("expected heavy for opcode 0x%02X", op)
		}
	}

	light := []cec.Opcode{cec.OpcodeReportPowerStatus, cec.OpcodeReportAudioStatus}
	for _, op := range light {
		if opcodeTopologyTier(op) != topologyLight {
			t.Errorf("expected light for opcode 0x%02X", op)
		}
	}

	none := []cec.Opcode{
		cec.OpcodeStandby,
		cec.OpcodeGivePhysicalAddress,
		cec.OpcodeFeatureAbort,
		cec.OpcodeUserControlPressed,
	}
	for _, op := range none {
		if opcodeTopologyTier(op) != topologyNone {
			t.Errorf("expected none for opcode 0x%02X", op)
		}
	}
}

func TestOpcodeRequestsTopologyRescan(t *testing.T) {
	// Any tier except none should trigger a topology hint.
	yes := []cec.Opcode{
		cec.OpcodeReportPhysicalAddress,
		cec.OpcodeReportPowerStatus,
		cec.OpcodeReportAudioStatus,
	}
	for _, op := range yes {
		if !opcodeRequestsTopologyRescan(op) {
			t.Errorf("expected true for opcode 0x%02X", op)
		}
	}

	no := []cec.Opcode{
		cec.OpcodeStandby,
		cec.OpcodeGivePhysicalAddress,
		cec.OpcodeFeatureAbort,
		cec.OpcodeUserControlPressed,
	}
	for _, op := range no {
		if opcodeRequestsTopologyRescan(op) {
			t.Errorf("expected false for opcode 0x%02X", op)
		}
	}
}
