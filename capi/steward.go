package main

import (
	"log"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/LukasParke/capi/cec"
)

// stewardJobsQueued / stewardJobsDropped feed /metrics; bumped every time
// enqueueSteward succeeds or fails respectively.
var (
	stewardJobsQueued  atomic.Uint64
	stewardJobsDropped atomic.Uint64
)

type stewardKind int

const (
	stewardNone stewardKind = iota
	stewardLight
	stewardFull
	stewardDeep
)

type stewardReq struct {
	kind stewardKind
	done chan struct{} // closed when finished; optional
}

var (
	busStewardCh      = make(chan stewardReq, 32)
	stewardStarted    sync.Once
	postCmdMu         sync.Mutex
	postCmdTimer      *time.Timer
	postCmdDebounce   = 400 * time.Millisecond
	stewardMonMu      sync.RWMutex
	stewardMonitoringOn bool
)

func startBusStewardIfNeeded() {
	stewardStarted.Do(func() {
		go runBusStewardLoop()
	})
}

func enqueueSteward(kind stewardKind, done chan struct{}) bool {
	req := stewardReq{kind: kind, done: done}
	select {
	case busStewardCh <- req:
		stewardJobsQueued.Add(1)
		return true
	default:
		stewardJobsDropped.Add(1)
		if done != nil {
			close(done)
		}
		log.Printf("bus steward: queue full, dropped job kind=%d", kind)
		return false
	}
}

func signalStewardFull() {
	startBusStewardIfNeeded()
	enqueueSteward(stewardFull, nil)
}

func signalStewardDeep(done chan struct{}) {
	startBusStewardIfNeeded()
	enqueueSteward(stewardDeep, done)
}

func signalStewardLight() {
	startBusStewardIfNeeded()
	enqueueSteward(stewardLight, nil)
}

func schedulePostCommandBusRefresh() {
	postCmdMu.Lock()
	defer postCmdMu.Unlock()
	if postCmdTimer != nil {
		postCmdTimer.Stop()
	}
	postCmdTimer = time.AfterFunc(postCmdDebounce, func() {
		signalStewardLight()
	})
}

func busConfigLocked() BusConfig {
	configMu.RLock()
	defer configMu.RUnlock()
	return currentConfig.Bus
}

func vendorProfileKey(vendorIDStr string) string {
	s := strings.TrimSpace(strings.ToUpper(vendorIDStr))
	if strings.HasPrefix(s, "0X") {
		return s
	}
	return s
}

func probeSkip(profile *VendorProfile, key string) bool {
	if profile == nil {
		return false
	}
	for _, k := range profile.SkipProbes {
		if strings.EqualFold(strings.TrimSpace(k), key) {
			return true
		}
	}
	return false
}

func runBusStewardLoop() {
	iv := busConfigLocked().reconcileInterval()
	timer := time.NewTimer(iv)
	defer timer.Stop()
	for {
		select {
		case req := <-busStewardCh:
			runStewardJob(req)
			if req.done != nil {
				close(req.done)
			}
		case <-timer.C:
			// Enqueue periodic work — never call runStewardJob inline here, or this goroutine
			// cannot read busStewardCh and HTTP-triggered scans stall until the timer job finishes.
			select {
			case busStewardCh <- stewardReq{kind: stewardFull, done: nil}:
			default:
				log.Printf("bus steward: periodic full reconcile skipped (queue full)")
			}
			niv := busConfigLocked().reconcileInterval()
			if niv != iv {
				iv = niv
			}
			if !timer.Stop() {
				select {
				case <-timer.C:
				default:
				}
			}
			timer.Reset(iv)
		}
	}
}

func runStewardJob(req stewardReq) {
	cfg := busConfigLocked()

	conn := adapter.Conn()
	if conn == nil {
		globalBusState.setCECReady(false)
		globalBusState.setScanInProgress(false)
		return
	}

	globalBusState.setScanInProgress(true)
	defer globalBusState.setScanInProgress(false)
	globalBusState.bumpGeneration()
	monitoring := stewardMonitoringEnabled()

	switch req.kind {
	case stewardDeep, stewardFull:
		// 1s baseline settle preserves the behavior that the cec package
		// used to apply unconditionally inside RescanDevices.
		settle := time.Second
		if req.kind == stewardDeep {
			settle = cfg.deepSettle()
		}
		if es := cfg.rescanExtraSettle(); es > 0 {
			settle += es
		}
		if err := conn.RescanDevices(settle); err != nil {
			appLog("steward", "RescanDevices: %v", err)
		}
	case stewardLight:
		// no RescanDevices
	}

	addrs := conn.LogicalAddressesWithOptionalPoll(true)
	activeSrc := -1
	if a, err := conn.GetActiveSource(); err == nil {
		activeSrc = int(a)
	}

	devMaps := make([]map[string]interface{}, 0, len(addrs))
	deadline := time.Now().Add(25 * time.Second)
	for _, addr := range addrs {
		if time.Now().After(deadline) {
			break
		}
		dev, err := conn.GetDeviceInfo(addr)
		if err != nil {
			continue
		}
		m := deviceToMap(dev)
		m["polled_at"] = time.Now().UTC().Format(time.RFC3339Nano)
		devMaps = append(devMaps, m)
	}

	if req.kind == stewardDeep {
		runGiveProbes(conn, devMaps, cfg)
	}

	now := time.Now()
	lastFull := now
	globalBusState.mergeObservedIntoDevices(devMaps)
	globalBusState.replaceSnapshot(
		devMaps,
		logicalAddrInts(addrs),
		activeSrc,
		true,
		monitoring,
		&lastFull,
		int(cfg.staleThreshold().Seconds()),
		cfg.frameRingSize(),
	)
	globalBusState.setScanInProgress(false)

	if eventHub != nil {
		eventHub.Publish(CECEvent{
			Type: "devices_changed",
			Data: map[string]interface{}{
				"reason":            "steward",
				"kind":              stewardKindString(req.kind),
				"logical_addresses": logicalAddrInts(addrs),
			},
		})
	}
	appLog("steward", "reconcile kind=%s addrs=%v devices=%d", stewardKindString(req.kind), logicalAddrInts(addrs), len(devMaps))
}

func stewardKindString(k stewardKind) string {
	switch k {
	case stewardLight:
		return "light"
	case stewardFull:
		return "full"
	case stewardDeep:
		return "deep"
	default:
		return "none"
	}
}

func logicalAddrInts(addrs []cec.LogicalAddress) []int {
	out := make([]int, len(addrs))
	for i, a := range addrs {
		out[i] = int(a)
	}
	return out
}

func stewardMonitoringEnabled() bool {
	stewardMonMu.RLock()
	defer stewardMonMu.RUnlock()
	return stewardMonitoringOn
}

func setStewardMonitoringState(on bool) {
	stewardMonMu.Lock()
	stewardMonitoringOn = on
	stewardMonMu.Unlock()
}

func runGiveProbes(conn *cec.Connection, devMaps []map[string]interface{}, cfg BusConfig) {
	const pause = 60 * time.Millisecond
	for _, dm := range devMaps {
		laInt, ok := dm["logical_address"].(int)
		if !ok {
			continue
		}
		la := cec.LogicalAddress(laInt)
		if la == cec.LogicalAddressBroadcast || la == cec.LogicalAddressFreeUse {
			continue
		}
		vidStr, _ := dm["vendor_id"].(string)
		var prof *VendorProfile
		if cfg.VendorProfiles != nil {
			if p, ok := cfg.VendorProfiles[vendorProfileKey(vidStr)]; ok {
				vp := p
				prof = &vp
			}
		}

		if !probeSkip(prof, "give_power_status") {
			_ = conn.GiveDevicePowerStatus(la)
			time.Sleep(pause)
		}
		if !probeSkip(prof, "give_osd_name") {
			_ = conn.GiveOSDName(la)
			time.Sleep(pause)
		}
		if !probeSkip(prof, "give_vendor_id") {
			_ = conn.GiveDeviceVendorID(la)
			time.Sleep(pause)
		}
		dt, _ := dm["device_type"].(string)
		if strings.Contains(strings.ToLower(dt), "playback") && !probeSkip(prof, "give_deck_status") {
			_ = conn.GiveDeckStatus(la, 3)
			time.Sleep(pause)
		}
		if strings.Contains(strings.ToLower(dt), "tuner") && !probeSkip(prof, "give_tuner_status") {
			_ = conn.GiveTunerDeviceStatus(la, 0x01)
			time.Sleep(pause)
		}
		if strings.Contains(strings.ToLower(dt), "audio") && !probeSkip(prof, "give_audio_status") {
			_ = conn.GiveAudioStatus(la)
			time.Sleep(pause)
			_ = conn.GiveSystemAudioModeStatus(la)
			time.Sleep(pause)
		}
		if !probeSkip(prof, "give_menu_language") {
			_ = conn.GiveMenuLanguage(la)
			time.Sleep(pause)
		}
		if !probeSkip(prof, "menu_request_query") {
			_ = conn.MenuRequest(la, 2)
			time.Sleep(pause)
		}
	}
	if !probeSkip(nil, "give_physical_address_broadcast") {
		_ = conn.GivePhysicalAddressBroadcast()
		time.Sleep(pause)
	}
}
