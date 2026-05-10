package main

import (
	"fmt"
	"net/http"
	"strconv"

	"github.com/LukasParke/capi/cec"

	"github.com/gorilla/mux"
)

// deviceToMap renders a cec.Device into the JSON shape used by /api/devices,
// /api/devices/{address}, and the steward snapshot.
func deviceToMap(dev *cec.Device) map[string]interface{} {
	hdmiPort := uint8(0)
	if dev.PhysicalAddress != 0 && dev.PhysicalAddress != 0xFFFF {
		hdmiPort = uint8((dev.PhysicalAddress >> 12) & 0xF)
	}
	return map[string]interface{}{
		"logical_address":  int(dev.LogicalAddress),
		"address_name":     dev.LogicalAddress.String(),
		"physical_address": cec.PhysicalAddressToString(dev.PhysicalAddress),
		"device_type":      cec.DeviceTypeForAddress(dev.LogicalAddress).String(),
		"hdmi_port":        int(hdmiPort),
		"vendor_id":        fmt.Sprintf("0x%06X", dev.VendorID),
		"vendor_name":      cec.GetVendorName(dev.VendorID),
		"cec_version":      dev.CECVersion.String(),
		"power_status":     dev.PowerStatus.String(),
		"osd_name":         dev.OSDName,
		"menu_language":    dev.MenuLanguage,
		"is_active":        dev.IsActive,
		"is_active_source": dev.IsActiveSource,
	}
}

// getDeviceHandler implements GET /api/devices/{address}.
func getDeviceHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	addr, err := strconv.Atoi(mux.Vars(r)["address"])
	if err != nil || addr < 0 || addr > 15 {
		respondError(w, http.StatusBadRequest, "Invalid logical address")
		return
	}

	c, ok := requireAdapter(func() {
		respondError(w, http.StatusServiceUnavailable, "CEC adapter not available")
	})
	if !ok {
		return
	}
	device, err := c.GetDeviceInfo(cec.LogicalAddress(addr))
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	appLog("devices", "GET /api/devices/%d ok", addr)
	respondSuccess(w, "Device info retrieved", deviceToMap(device))
}

// getBusStateHandler implements GET /api/bus/state. Returns the steward's
// cached snapshot, never blocks.
func getBusStateHandler(w http.ResponseWriter, r *http.Request) {
	ready := adapterReady()
	snap := globalBusState.copySnapshot()
	snap.CECReady = ready
	if !ready {
		snap.Stale = true
	}
	respondSuccess(w, "Bus state", map[string]interface{}{
		"updated_at":          snap.UpdatedAt,
		"scan_generation":     snap.ScanGeneration,
		"last_full_scan_at":   snap.LastFullScanAt,
		"scan_in_progress":    snap.ScanInProgress,
		"stale":               snap.Stale,
		"stale_threshold_sec": snap.StaleThresholdSec,
		"cec_ready":           snap.CECReady,
		"monitoring":          snap.Monitoring,
		"active_source":       snap.ActiveSource,
		"logical_addresses":   snap.LogicalAddresses,
		"devices":             snap.Devices,
		"frame_ring_size":     snap.FrameRingSize,
		"recent_frames":       snap.RecentFrames,
	})
}

// postBusScanHandler implements POST /api/bus/scan. Queues a deep scan and
// returns 202 Accepted with a poll URL; the client watches /api/bus/state
// (and the SSE devices_changed event) until scan_in_progress flips to false.
func postBusScanHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	if !enqueueSteward(stewardDeep, nil) {
		respondError(w, http.StatusServiceUnavailable, "Bus steward queue full")
		return
	}
	w.Header().Set("Location", "/api/bus/state")
	respondJSON(w, http.StatusAccepted, Response{
		Status:  "accepted",
		Message: "Deep bus scan queued (Give* probes + extended settle); poll GET /api/bus/state until scan_in_progress is false",
		Data: map[string]interface{}{
			"poll": "/api/bus/state",
		},
	})
}

// getBusFramesHandler implements GET /api/bus/frames. Returns the most recent
// raw frames captured when the frame ring is enabled.
func getBusFramesHandler(w http.ResponseWriter, r *http.Request) {
	snap := globalBusState.copySnapshot()
	if len(snap.RecentFrames) == 0 {
		respondSuccess(w, "No frames captured (enable bus.frame_ring_size in config or -cec-monitor)", []BusFrameEntry{})
		return
	}
	respondSuccess(w, "Recent CEC frames", snap.RecentFrames)
}

// getTopologyHandler implements GET /api/topology.
func getTopologyHandler(w http.ResponseWriter, r *http.Request) {
	c, ok := requireAdapter(func() {
		respondError(w, http.StatusServiceUnavailable, "CEC adapter not available")
	})
	if !ok {
		return
	}
	p := buildTopologyPayloadLocked(c)

	appLog("topology", "GET /api/topology own=%v own_port=%d ports=%d", p.OwnAddresses, p.OwnPort, len(p.Ports))
	ports := make([]map[string]interface{}, len(p.Ports))
	for i, row := range p.Ports {
		ports[i] = map[string]interface{}{
			"port":    row.Port,
			"devices": row.Devices,
		}
	}
	respondSuccess(w, "Bus topology retrieved", map[string]interface{}{
		"own_addresses":    p.OwnAddresses,
		"own_port":         p.OwnPort,
		"known_port_count": p.KnownPortCount,
		"active_ports":     ports,
	})
}
