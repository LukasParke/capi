package main

import (
	"strconv"
	"testing"
	"time"

	"github.com/LukasParke/capi/cec"
)

func BenchmarkEventHubPublish_NoSubscribers(b *testing.B) {
	hub := NewEventHub(64)
	ev := CECEvent{Type: "command", Data: map[string]interface{}{"opcode": "0x90"}}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		hub.Publish(ev)
	}
}

func BenchmarkEventHubPublish_OneSubscriber(b *testing.B) {
	hub := NewEventHub(1024)
	ch := hub.Subscribe()
	defer hub.Unsubscribe(ch)
	go func() {
		for range ch {
		}
	}()
	ev := CECEvent{Type: "command", Data: map[string]interface{}{"opcode": "0x90"}}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		hub.Publish(ev)
	}
}

func BenchmarkEventHubPublish_FourSubscribers(b *testing.B) {
	hub := NewEventHub(1024)
	for i := 0; i < 4; i++ {
		ch := hub.Subscribe()
		go func(c chan CECEvent) {
			for range c {
			}
		}(ch)
	}
	ev := CECEvent{Type: "command", Data: map[string]interface{}{"opcode": "0x90"}}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		hub.Publish(ev)
	}
}

// BenchmarkEventHubPublish_SlowSubscriber simulates the worst case the
// non-blocking publisher must handle: a subscriber whose channel is full
// every time. Each publish is O(N) over subscribers and lands in the dropped
// counter.
func BenchmarkEventHubPublish_SlowSubscriber(b *testing.B) {
	hub := NewEventHub(1)
	ch := hub.Subscribe()
	defer hub.Unsubscribe(ch)
	hub.Publish(CECEvent{Type: "log"}) // fill the slot
	ev := CECEvent{Type: "command"}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		hub.Publish(ev)
	}
}

// BenchmarkMergeObservedIntoDevices measures the device-list merge that runs
// at the end of every steward job. The race-free version copies the
// observed-by-address map under the lock; this benchmark verifies the cost
// stays low when there are a realistic number of devices and observations.
func BenchmarkMergeObservedIntoDevices(b *testing.B) {
	store := &busStateStore{
		observedByAddr: make(map[int]map[string]interface{}),
	}
	for la := 0; la < 8; la++ {
		store.observedByAddr[la] = map[string]interface{}{
			"observed_power_status": "on",
			"observed_vendor_id":    "0x000039",
			"observed_vendor_name":  "Toshiba",
			"observed_at":           time.Now().Format(time.RFC3339Nano),
		}
	}
	devices := make([]map[string]interface{}, 8)
	for i := range devices {
		devices[i] = map[string]interface{}{
			"logical_address": i,
			"osd_name":        "Device" + strconv.Itoa(i),
		}
	}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		// Copy devices each iteration so the merge writes on a fresh slice
		// (the production caller does the same after every steward rebuild).
		dst := make([]map[string]interface{}, len(devices))
		for j, d := range devices {
			dst[j] = make(map[string]interface{}, len(d))
			for k, v := range d {
				dst[j][k] = v
			}
		}
		store.mergeObservedIntoDevices(dst)
	}
}

// BenchmarkRecordObserved measures the per-bus-frame fast path used when
// the cec event consumer sees ReportPowerStatus / DeviceVendorID / etc.
func BenchmarkRecordObserved(b *testing.B) {
	store := &busStateStore{observedByAddr: make(map[int]map[string]interface{})}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		store.recordObserved(i&0xF, "observed_power_status", "on")
	}
}

// BenchmarkApplyObservedFromCECCommand measures the full classification
// path from a parsed cec.Command to the busStateStore observed map. This
// runs on every CEC frame seen by the event consumer.
func BenchmarkApplyObservedFromCECCommand(b *testing.B) {
	store := &busStateStore{observedByAddr: make(map[int]map[string]interface{})}
	cmd := &cec.Command{
		Initiator:   cec.LogicalAddressPlaybackDevice1,
		Destination: cec.LogicalAddressBroadcast,
		Opcode:      cec.OpcodeReportPowerStatus,
		Parameters:  []uint8{0x00},
	}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		store.ApplyObservedFromCECCCommand(cmd)
	}
}

// BenchmarkAppendFrameRing measures the per-frame cost when the frame ring
// is enabled (cec-monitor mode).
func BenchmarkAppendFrameRing(b *testing.B) {
	store := &busStateStore{}
	cmd := &cec.Command{
		Initiator:   cec.LogicalAddressPlaybackDevice1,
		Destination: cec.LogicalAddressBroadcast,
		Opcode:      cec.OpcodeReportPowerStatus,
		Parameters:  []uint8{0x00, 0x01, 0x02},
	}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		store.appendFrameRing(cmd, 1024)
	}
}

// BenchmarkOpcodeTopologyTier covers the per-frame classifier that decides
// whether to nudge the steward.
func BenchmarkOpcodeTopologyTier(b *testing.B) {
	ops := []cec.Opcode{
		cec.OpcodeReportPhysicalAddress,
		cec.OpcodeDeviceVendorID,
		cec.OpcodeReportPowerStatus,
		cec.OpcodeImageViewOn,
	}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = opcodeTopologyTier(ops[i&3])
	}
}

// BenchmarkAdapterConn measures the atomic.Pointer load that every CEC
// helper executes. This used to be a sync.Mutex Lock/Unlock around cecConn.
func BenchmarkAdapterConn(b *testing.B) {
	a := NewAdapter()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = a.Conn()
	}
}

// BenchmarkCopySnapshot measures the deep copy that every UI fragment +
// SSE handler does to read the current bus state.
func BenchmarkCopySnapshot(b *testing.B) {
	store := &busStateStore{
		snap: BusStateSnapshot{
			LogicalAddresses: []int{0, 1, 4, 5},
			Devices:          make([]map[string]interface{}, 4),
		},
	}
	for i := range store.snap.Devices {
		store.snap.Devices[i] = map[string]interface{}{
			"logical_address": i,
			"osd_name":        "Device",
			"power_status":    "on",
			"vendor_id":       "0x000000",
		}
	}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = store.copySnapshot()
	}
}
