package cec

import "testing"

func TestLogicalAddressClassification(t *testing.T) {
	cases := []struct {
		la                LogicalAddress
		valid, broadcast, unknown bool
	}{
		{LogicalAddressTV, true, false, false},
		{LogicalAddressPlaybackDevice1, true, false, false},
		{LogicalAddressFreeUse, true, false, false},
		{LogicalAddressBroadcast, false, true, false},
		{LogicalAddressUnknown, false, false, true},
	}
	for _, c := range cases {
		if got := c.la.IsValid(); got != c.valid {
			t.Errorf("%v IsValid = %v, want %v", c.la, got, c.valid)
		}
		if got := c.la.IsBroadcast(); got != c.broadcast {
			t.Errorf("%v IsBroadcast = %v, want %v", c.la, got, c.broadcast)
		}
		if got := c.la.IsUnknown(); got != c.unknown {
			t.Errorf("%v IsUnknown = %v, want %v", c.la, got, c.unknown)
		}
	}
}

func TestLogicalAddressBroadcastUnknownDistinct(t *testing.T) {
	// Broadcast (0xF) and Unknown (0xFF) used to collide at the same value;
	// guard against a regression.
	if LogicalAddressBroadcast == LogicalAddressUnknown {
		t.Fatalf("broadcast and unknown must be distinct values")
	}
}

func TestLogicalAddressString(t *testing.T) {
	cases := map[LogicalAddress]string{
		LogicalAddressTV:              "TV",
		LogicalAddressAudioSystem:     "Audio System",
		LogicalAddressPlaybackDevice1: "Playback Device 1",
		LogicalAddressBroadcast:       "Broadcast",
		LogicalAddressUnknown:         "Unknown",
	}
	for la, want := range cases {
		if got := la.String(); got != want {
			t.Errorf("%v.String() = %q, want %q", la, got, want)
		}
	}
}

func TestDeviceTypeString(t *testing.T) {
	cases := map[DeviceType]string{
		DeviceTypeTV:              "TV",
		DeviceTypeRecordingDevice: "Recording Device",
		DeviceTypeTuner:           "Tuner",
		DeviceTypePlaybackDevice:  "Playback Device",
		DeviceTypeAudioSystem:     "Audio System",
		DeviceTypeReserved:        "Reserved",
	}
	for dt, want := range cases {
		if got := dt.String(); got != want {
			t.Errorf("%v.String() = %q, want %q", dt, got, want)
		}
	}
}

func TestPowerStatusString(t *testing.T) {
	cases := map[PowerStatus]string{
		PowerStatusOn:                      "On",
		PowerStatusStandby:                 "Standby",
		PowerStatusInTransitionStandbyToOn: "Transitioning to On",
		PowerStatusInTransitionOnToStandby: "Transitioning to Standby",
		PowerStatusUnknown:                 "Unknown",
	}
	for p, want := range cases {
		if got := p.String(); got != want {
			t.Errorf("%v.String() = %q, want %q", p, got, want)
		}
	}
}

func TestLogLevelString(t *testing.T) {
	cases := map[LogLevel]string{
		LogLevelError:   "ERROR",
		LogLevelWarning: "WARNING",
		LogLevelNotice:  "NOTICE",
		LogLevelTraffic: "TRAFFIC",
		LogLevelDebug:   "DEBUG",
		LogLevelAll:     "ALL",
	}
	for ll, want := range cases {
		if got := ll.String(); got != want {
			t.Errorf("%v.String() = %q, want %q", ll, got, want)
		}
	}
}

func TestAlertString(t *testing.T) {
	if AlertConnectionLost.String() != "ConnectionLost" {
		t.Errorf("AlertConnectionLost.String() unexpected: %q", AlertConnectionLost.String())
	}
	if Alert(99).String() != "Unknown" {
		t.Errorf("unknown alert should stringify as Unknown")
	}
}

func TestEventKindString(t *testing.T) {
	if EventCommand.String() != "command" {
		t.Errorf("EventCommand.String() = %q", EventCommand.String())
	}
	if EventInvalid.String() != "invalid" {
		t.Errorf("EventInvalid.String() = %q", EventInvalid.String())
	}
}
