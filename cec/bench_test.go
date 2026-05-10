package cec

import (
	"testing"
	"time"
)

func BenchmarkGetVendorNameKnown(b *testing.B) {
	for i := 0; i < b.N; i++ {
		_ = GetVendorName(0x000039)
	}
}

func BenchmarkGetVendorNameUnknown(b *testing.B) {
	for i := 0; i < b.N; i++ {
		_ = GetVendorName(0xABCDEF)
	}
}

func BenchmarkPhysicalAddressToString(b *testing.B) {
	for i := 0; i < b.N; i++ {
		_ = PhysicalAddressToString(0x2100)
	}
}

func BenchmarkParsePhysicalAddress(b *testing.B) {
	for i := 0; i < b.N; i++ {
		_, _ = ParsePhysicalAddress("2.1.0.0")
	}
}

// BenchmarkDispatchHotChannel measures the per-event cost of dispatch when
// the consumer is keeping up (channel never blocks). This is the hottest
// path in the cec event pipeline.
func BenchmarkDispatchHotChannel(b *testing.B) {
	c := &Connection{events: make(chan Event, 1024)}
	done := make(chan struct{})
	go func() {
		defer close(done)
		for range c.events {
		}
	}()

	ev := Event{Kind: EventCommand, Command: &Command{Opcode: OpcodeReportPowerStatus}}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		c.dispatch(ev)
	}
	b.StopTimer()
	close(c.events)
	<-done
}

// BenchmarkDispatchFullChannel measures the dispatch cost when every event
// is dropped (channel is full). This bounds the worst-case overhead of the
// non-blocking publish path.
func BenchmarkDispatchFullChannel(b *testing.B) {
	c := &Connection{events: make(chan Event, 1)}
	c.dispatch(Event{Kind: EventLog})
	ev := Event{Kind: EventCommand}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		c.dispatch(ev)
	}
}

// BenchmarkDeviceTypeForAddress is a tiny switch that runs on every scan.
func BenchmarkDeviceTypeForAddress(b *testing.B) {
	for i := 0; i < b.N; i++ {
		_ = DeviceTypeForAddress(LogicalAddressPlaybackDevice1)
	}
}

// BenchmarkEventStringRoundtrip exercises the EventKind/Alert/PowerStatus
// String paths used during event serialization.
func BenchmarkEventStringRoundtrip(b *testing.B) {
	for i := 0; i < b.N; i++ {
		_ = EventCommand.String()
		_ = AlertConnectionLost.String()
		_ = PowerStatusOn.String()
	}
}

// keep time import used in case timestamp is added later
var _ = time.Now
