package cec

import (
	"strings"
	"testing"
)

func TestPhysicalAddressRoundTrip(t *testing.T) {
	cases := []struct {
		raw uint16
		dot string
	}{
		{0x0000, "0.0.0.0"},
		{0x1000, "1.0.0.0"},
		{0x2100, "2.1.0.0"},
		{0xFFFF, "15.15.15.15"},
	}
	for _, c := range cases {
		got := PhysicalAddressToString(c.raw)
		if got != c.dot {
			t.Errorf("PhysicalAddressToString(%#04x) = %q, want %q", c.raw, got, c.dot)
		}
		back, err := ParsePhysicalAddress(c.dot)
		if err != nil {
			t.Fatalf("ParsePhysicalAddress(%q): %v", c.dot, err)
		}
		if back != c.raw {
			t.Errorf("ParsePhysicalAddress(%q) = %#04x, want %#04x", c.dot, back, c.raw)
		}
	}
}

func TestParsePhysicalAddressInvalid(t *testing.T) {
	cases := []string{"", "1.2.3", "1.2.3.4.5", "16.0.0.0", "0.0.0.16", "abc"}
	for _, in := range cases {
		if _, err := ParsePhysicalAddress(in); err == nil {
			t.Errorf("ParsePhysicalAddress(%q) expected error, got nil", in)
		}
	}
}

func TestDeviceTypeForAddress(t *testing.T) {
	cases := map[LogicalAddress]DeviceType{
		LogicalAddressTV:               DeviceTypeTV,
		LogicalAddressRecordingDevice1: DeviceTypeRecordingDevice,
		LogicalAddressRecordingDevice2: DeviceTypeRecordingDevice,
		LogicalAddressRecordingDevice3: DeviceTypeRecordingDevice,
		LogicalAddressTuner1:           DeviceTypeTuner,
		LogicalAddressTuner4:           DeviceTypeTuner,
		LogicalAddressPlaybackDevice1:  DeviceTypePlaybackDevice,
		LogicalAddressPlaybackDevice3:  DeviceTypePlaybackDevice,
		LogicalAddressAudioSystem:      DeviceTypeAudioSystem,
		LogicalAddressBroadcast:        DeviceTypeReserved,
	}
	for la, want := range cases {
		if got := DeviceTypeForAddress(la); got != want {
			t.Errorf("DeviceTypeForAddress(%v) = %v, want %v", la, got, want)
		}
	}
}

func TestGetVendorNameKnown(t *testing.T) {
	if GetVendorName(0x000039) != "Toshiba" {
		t.Errorf("Toshiba mapping wrong")
	}
	if GetVendorName(0x001582) != "Pulse Eight" {
		t.Errorf("Pulse Eight mapping wrong")
	}
}

func TestGetVendorNameUnknown(t *testing.T) {
	got := GetVendorName(0xABCDEF)
	if !strings.HasPrefix(got, "Unknown ") || !strings.Contains(got, "0xABCDEF") {
		t.Errorf("unknown vendor format unexpected: %q", got)
	}
}

func TestVolumePassThroughOrder(t *testing.T) {
	order := volumePassThroughOrder()
	if len(order) == 0 {
		t.Fatal("volume order is empty")
	}
	if order[0] != LogicalAddressAudioSystem {
		t.Errorf("first preference should be AudioSystem, got %v", order[0])
	}
	// AudioSystem should appear before TV (we only consult TV as a fallback).
	avIdx, tvIdx := -1, -1
	for i, la := range order {
		if la == LogicalAddressAudioSystem {
			avIdx = i
		}
		if la == LogicalAddressTV {
			tvIdx = i
		}
	}
	if avIdx >= tvIdx {
		t.Errorf("AudioSystem (%d) should precede TV (%d)", avIdx, tvIdx)
	}
}

func TestDeviceInfoErrorsAggregation(t *testing.T) {
	var d DeviceInfoErrors
	if d.Any() || d.All() {
		t.Errorf("zero DeviceInfoErrors should be neither Any nor All")
	}
	d.OSDName = ErrLibcecCall
	if !d.Any() {
		t.Errorf("Any should be true when one field is set")
	}
	if d.All() {
		t.Errorf("All should be false when only one field is set")
	}
	d.PhysicalAddress = ErrLibcecCall
	d.VendorID = ErrLibcecCall
	d.CECVersion = ErrLibcecCall
	d.PowerStatus = ErrLibcecCall
	d.MenuLanguage = ErrLibcecCall
	if !d.All() {
		t.Errorf("All should be true when every field is set")
	}
	msg := d.Error()
	if !strings.Contains(msg, "physical:") || !strings.Contains(msg, "osd:") {
		t.Errorf("Error message missing field tags: %q", msg)
	}
}
