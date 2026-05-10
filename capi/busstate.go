package main

import (
	"fmt"
	"sync"
	"time"

	"github.com/LukasParke/capi/cec"
)

// BusFrameEntry is one captured CEC frame (when frame ring is enabled).
type BusFrameEntry struct {
	Timestamp   time.Time `json:"timestamp"`
	Initiator   int       `json:"initiator"`
	Destination int       `json:"destination"`
	Opcode      string    `json:"opcode"`
	Ack         bool      `json:"ack"`
	EOM         bool      `json:"eom"`
	OpcodeSet   bool      `json:"opcode_set"`
	ParamsHex   []string  `json:"params_hex"`
}

// VendorProfile tweaks probe behavior for a vendor ID key "0xABCDEF".
type VendorProfile struct {
	SkipProbes []string `json:"skip_probes"` // e.g. "give_audio_status", "give_deck_status"
	SettleMs   int      `json:"settle_ms"`   // extra post-rescan settle for this vendor (0=default)
}

// BusConfig is persisted under config.json "bus".
type BusConfig struct {
	ReconcileIntervalSec int                       `json:"reconcile_interval_sec"`
	DeepSettleMs         int                       `json:"deep_settle_ms"`
	RescanExtraSettleMs  int                       `json:"rescan_extra_settle_ms"`
	StaleThresholdSec    int                       `json:"stale_threshold_sec"`
	FrameRingSize        int                       `json:"frame_ring_size"`
	MonitorFromConfig    *bool                     `json:"monitor,omitempty"` // nil = use CLI only; true/false override
	VendorProfiles       map[string]VendorProfile `json:"vendor_profiles"`
}

func (b BusConfig) reconcileInterval() time.Duration {
	if b.ReconcileIntervalSec <= 0 {
		return 60 * time.Second
	}
	return time.Duration(b.ReconcileIntervalSec) * time.Second
}

func (b BusConfig) deepSettle() time.Duration {
	if b.DeepSettleMs <= 0 {
		return 2500 * time.Millisecond
	}
	return time.Duration(b.DeepSettleMs) * time.Millisecond
}

func (b BusConfig) rescanExtraSettle() time.Duration {
	if b.RescanExtraSettleMs < 0 {
		return 0
	}
	return time.Duration(b.RescanExtraSettleMs) * time.Millisecond
}

func (b BusConfig) staleThreshold() time.Duration {
	if b.StaleThresholdSec <= 0 {
		return 180 * time.Second
	}
	return time.Duration(b.StaleThresholdSec) * time.Second
}

func (b BusConfig) frameRingSize() int {
	if b.FrameRingSize < 0 {
		return 0
	}
	return b.FrameRingSize
}

// BusStateSnapshot is returned by GET /api/bus/state (JSON-friendly).
type BusStateSnapshot struct {
	UpdatedAt          time.Time                `json:"updated_at"`
	ScanGeneration     int64                    `json:"scan_generation"`
	LastFullScanAt     *time.Time               `json:"last_full_scan_at,omitempty"`
	ScanInProgress     bool                     `json:"scan_in_progress"`
	Stale              bool                     `json:"stale"`
	StaleThresholdSec  int                      `json:"stale_threshold_sec"`
	CECReady           bool                     `json:"cec_ready"`
	Monitoring         bool                     `json:"monitoring"`
	ActiveSource       int                      `json:"active_source"` // -1 if unknown
	LogicalAddresses   []int                    `json:"logical_addresses"`
	Devices            []map[string]interface{} `json:"devices"`
	FrameRingSize      int                      `json:"frame_ring_size"`
	RecentFrames       []BusFrameEntry          `json:"recent_frames,omitempty"`
}

// busStateStore holds the latest bus snapshot and optional frame ring.
type busStateStore struct {
	mu             sync.RWMutex
	snap           BusStateSnapshot
	frameRing      []BusFrameEntry
	frameRingCap   int
	observedByAddr map[int]map[string]interface{} // merged into devices on each rebuild
}

var globalBusState = &busStateStore{
	observedByAddr: make(map[int]map[string]interface{}),
	snap: BusStateSnapshot{
		ActiveSource: -1,
	},
}

func (s *busStateStore) setMonitoring(on bool) {
	s.mu.Lock()
	s.snap.Monitoring = on
	s.mu.Unlock()
}

func (s *busStateStore) setCECReady(ready bool) {
	s.mu.Lock()
	s.snap.CECReady = ready
	if !ready {
		s.snap.Devices = nil
		s.snap.LogicalAddresses = nil
		s.snap.ActiveSource = -1
		s.observedByAddr = make(map[int]map[string]interface{})
	}
	s.mu.Unlock()
}

func (s *busStateStore) setScanInProgress(v bool) {
	s.mu.Lock()
	s.snap.ScanInProgress = v
	s.mu.Unlock()
}

func (s *busStateStore) bumpGeneration() int64 {
	s.mu.Lock()
	s.snap.ScanGeneration++
	gen := s.snap.ScanGeneration
	s.mu.Unlock()
	return gen
}

func (s *busStateStore) replaceSnapshot(devices []map[string]interface{}, addrs []int, activeSource int, cecReady, monitoring bool, lastFull *time.Time, staleThresholdSec int, frameCap int) {
	s.mu.Lock()
	now := time.Now()
	s.snap.UpdatedAt = now
	s.snap.Devices = devices
	s.snap.LogicalAddresses = addrs
	s.snap.ActiveSource = activeSource
	s.snap.CECReady = cecReady
	s.snap.Monitoring = monitoring
	s.snap.LastFullScanAt = lastFull
	s.snap.StaleThresholdSec = staleThresholdSec
	if lastFull != nil {
		th := time.Duration(staleThresholdSec) * time.Second
		s.snap.Stale = now.Sub(*lastFull) > th
	} else {
		s.snap.Stale = true
	}
	s.snap.FrameRingSize = frameCap
	if frameCap > 0 && len(s.frameRing) > 0 {
		s.snap.RecentFrames = append([]BusFrameEntry(nil), s.frameRing...)
	} else {
		s.snap.RecentFrames = nil
	}
	s.mu.Unlock()
}

func (s *busStateStore) updateActiveSourceQuick(active int, cecReady bool) {
	s.mu.Lock()
	s.snap.ActiveSource = active
	s.snap.CECReady = cecReady
	s.snap.UpdatedAt = time.Now()
	s.mu.Unlock()
}

func (s *busStateStore) copySnapshot() BusStateSnapshot {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := s.snap
	out.Devices = deepCopyDeviceMaps(s.snap.Devices)
	out.LogicalAddresses = append([]int(nil), s.snap.LogicalAddresses...)
	if len(s.snap.RecentFrames) > 0 {
		out.RecentFrames = append([]BusFrameEntry(nil), s.snap.RecentFrames...)
	}
	return out
}

func deepCopyDeviceMaps(in []map[string]interface{}) []map[string]interface{} {
	if len(in) == 0 {
		return nil
	}
	out := make([]map[string]interface{}, len(in))
	for i, m := range in {
		out[i] = map[string]interface{}{}
		for k, v := range m {
			out[i][k] = v
		}
	}
	return out
}

// mergeObservedIntoDevices folds passively observed bus traffic (recordObserved)
// into the steward-built device maps. It takes a snapshot of observedByAddr
// under the lock and copies only the per-address inner maps it needs, so the
// subsequent iteration is race-free against ongoing recordObserved writes.
func (s *busStateStore) mergeObservedIntoDevices(devices []map[string]interface{}) []map[string]interface{} {
	if len(devices) == 0 {
		return devices
	}

	// Build the set of addresses we'll need outside the lock.
	wanted := make(map[int]struct{}, len(devices))
	for i := range devices {
		if la, ok := devices[i]["logical_address"].(int); ok {
			wanted[la] = struct{}{}
		}
	}

	// Snapshot only the inner maps we care about, under the store mutex.
	snap := make(map[int]map[string]interface{}, len(wanted))
	s.mu.Lock()
	for la := range wanted {
		o := s.observedByAddr[la]
		if len(o) == 0 {
			continue
		}
		copyOf := make(map[string]interface{}, len(o))
		for k, v := range o {
			copyOf[k] = v
		}
		snap[la] = copyOf
	}
	s.mu.Unlock()

	if len(snap) == 0 {
		return devices
	}
	for i := range devices {
		la, ok := devices[i]["logical_address"].(int)
		if !ok {
			continue
		}
		o, ok := snap[la]
		if !ok {
			continue
		}
		for k, v := range o {
			devices[i][k] = v
		}
	}
	return devices
}

func (s *busStateStore) recordObserved(addr int, key string, value interface{}) {
	s.mu.Lock()
	if s.observedByAddr[addr] == nil {
		s.observedByAddr[addr] = make(map[string]interface{})
	}
	s.observedByAddr[addr][key] = value
	s.observedByAddr[addr]["observed_at"] = time.Now().UTC().Format(time.RFC3339Nano)
	s.mu.Unlock()
}

func uint16FromParamsBE(p []uint8) (uint16, bool) {
	if len(p) < 2 {
		return 0, false
	}
	return uint16(p[0])<<8 | uint16(p[1]), true
}

// ApplyObservedFromCECCCommand updates passive "observed_*" fields from bus traffic (no cecMutex).
func (s *busStateStore) ApplyObservedFromCECCCommand(cmd *cec.Command) {
	if cmd == nil {
		return
	}
	initiator := int(cmd.Initiator)
	switch cmd.Opcode {
	case cec.OpcodeReportPhysicalAddress:
		if phys, ok := uint16FromParamsBE(cmd.Parameters); ok && len(cmd.Parameters) >= 3 {
			s.recordObserved(initiator, "observed_physical_address", cec.PhysicalAddressToString(phys))
			s.recordObserved(initiator, "observed_device_type", int(cmd.Parameters[2]))
		}
	case cec.OpcodeDeviceVendorID:
		if len(cmd.Parameters) >= 3 {
			vid := uint64(cmd.Parameters[0])<<16 | uint64(cmd.Parameters[1])<<8 | uint64(cmd.Parameters[2])
			s.recordObserved(initiator, "observed_vendor_id", fmt.Sprintf("0x%06X", vid))
			s.recordObserved(initiator, "observed_vendor_name", cec.GetVendorName(vid))
		}
	case cec.OpcodeReportPowerStatus:
		if len(cmd.Parameters) >= 1 {
			s.recordObserved(initiator, "observed_power_status", powerStatusFromByte(cmd.Parameters[0]))
		}
	case cec.OpcodeReportAudioStatus:
		if len(cmd.Parameters) >= 1 {
			b := cmd.Parameters[0]
			s.recordObserved(initiator, "observed_audio_muted", (b&0x80) != 0)
			s.recordObserved(initiator, "observed_audio_volume_raw", int(b&0x7F))
		}
	case cec.OpcodeActiveSource:
		if phys, ok := uint16FromParamsBE(cmd.Parameters); ok {
			s.recordObserved(initiator, "observed_active_source_physical", cec.PhysicalAddressToString(phys))
		}
	case cec.OpcodeSetOSDName:
		if len(cmd.Parameters) > 0 {
			// First byte is start segment index; rest is ASCII name chunk (simplified).
			name := string(cmd.Parameters[1:])
			if name != "" {
				s.recordObserved(initiator, "observed_osd_name_fragment", name)
			}
		}
	case cec.OpcodeRoutingInformation, cec.OpcodeSetStreamPath:
		if phys, ok := uint16FromParamsBE(cmd.Parameters); ok {
			s.recordObserved(initiator, "observed_routing_physical", cec.PhysicalAddressToString(phys))
		}
	case cec.OpcodeDeckStatus:
		if len(cmd.Parameters) >= 1 {
			s.recordObserved(initiator, "observed_deck_status", int(cmd.Parameters[0]))
		}
	case cec.OpcodeTunerDeviceStatus:
		if len(cmd.Parameters) >= 1 {
			s.recordObserved(initiator, "observed_tuner_status_hex", fmt.Sprintf("%x", cmd.Parameters))
		}
	case cec.OpcodeMenuStatus:
		if len(cmd.Parameters) >= 1 {
			s.recordObserved(initiator, "observed_menu_state", int(cmd.Parameters[0]))
		}
	case cec.OpcodeSystemAudioModeStatus:
		if len(cmd.Parameters) >= 1 {
			s.recordObserved(initiator, "observed_system_audio_mode", int(cmd.Parameters[0]))
		}
	case cec.OpcodeFeatureAbort:
		if len(cmd.Parameters) >= 2 {
			s.recordObserved(initiator, "observed_last_feature_abort_opcode", int(cmd.Parameters[0]))
			s.recordObserved(initiator, "observed_last_feature_abort_reason", int(cmd.Parameters[1]))
		}
	}
}

func (s *busStateStore) appendFrameRing(cmd *cec.Command, cap int) {
	if cap <= 0 || cmd == nil {
		return
	}
	params := make([]string, len(cmd.Parameters))
	for i, b := range cmd.Parameters {
		params[i] = fmt.Sprintf("%02X", b)
	}
	ent := BusFrameEntry{
		Timestamp:   time.Now().UTC(),
		Initiator:   int(cmd.Initiator),
		Destination: int(cmd.Destination),
		Opcode:      fmt.Sprintf("0x%02X", cmd.Opcode),
		Ack:         cmd.Ack,
		EOM:         cmd.Eom,
		OpcodeSet:   cmd.OpcodeSet,
		ParamsHex:   params,
	}
	s.mu.Lock()
	s.frameRing = append(s.frameRing, ent)
	if len(s.frameRing) > cap {
		s.frameRing = s.frameRing[len(s.frameRing)-cap:]
	}
	s.mu.Unlock()
}

func (s *busStateStore) setFrameRingCapacity(cap int) {
	s.mu.Lock()
	s.frameRingCap = cap
	if cap <= 0 {
		s.frameRing = nil
	}
	s.mu.Unlock()
}
