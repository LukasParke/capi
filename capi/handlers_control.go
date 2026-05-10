package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"
	"strings"

	"github.com/LukasParke/capi/cec"

	"github.com/gorilla/mux"
)

// optionalAddrParam parses an optional {address} URL parameter; returns
// (def, false, nil) if absent, (addr, true, nil) on success, or
// (0, false, err) on a malformed value.
func optionalAddrParam(r *http.Request, def int) (int, bool, error) {
	addrStr := strings.TrimSpace(mux.Vars(r)["address"])
	if addrStr == "" {
		return def, false, nil
	}
	addr, err := strconv.Atoi(addrStr)
	if err != nil || addr < 0 || addr > 15 {
		return 0, false, fmt.Errorf("Invalid logical address")
	}
	return addr, true, nil
}

// powerOnHandler implements POST /api/power/on[/{address}].
func powerOnHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	addr, _, err := optionalAddrParam(r, 0)
	if err != nil {
		respondError(w, http.StatusBadRequest, err.Error())
		return
	}
	if err := execPowerOn(addr); err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, fmt.Sprintf("Power on command sent to device %d", addr), nil)
}

// powerOffHandler implements POST /api/power/off[/{address}].
func powerOffHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	addr, _, err := optionalAddrParam(r, 0)
	if err != nil {
		respondError(w, http.StatusBadRequest, err.Error())
		return
	}
	if err := execPowerOff(addr); err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, fmt.Sprintf("Standby command sent to device %d", addr), nil)
}

// getPowerStatusHandler implements GET /api/power/status[/{address}].
func getPowerStatusHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	addr, _, err := optionalAddrParam(r, 0)
	if err != nil {
		respondError(w, http.StatusBadRequest, err.Error())
		return
	}
	status, err := execPowerStatus(addr)
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, "Power status retrieved", map[string]interface{}{
		"address": addr,
		"status":  status,
	})
}

// volumeUpHandler implements POST /api/volume/up[/{address}].
func volumeUpHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	addrStr := mux.Vars(r)["address"]
	if addrStr != "" {
		appLog("volume", "POST /api/volume/up/%s", addrStr)
	} else {
		appLog("volume", "POST /api/volume/up best-effort (SAM nudge then libcec)")
	}
	msg, err := execVolumeUp(addrStr)
	if err != nil {
		appLog("volume", "POST /api/volume/up failed: %v", err)
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, msg, nil)
}

// volumeDownHandler implements POST /api/volume/down[/{address}].
func volumeDownHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	msg, err := execVolumeDown(mux.Vars(r)["address"])
	if err != nil {
		appLog("volume", "POST /api/volume/down failed: %v", err)
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, msg, nil)
}

// muteHandler implements POST /api/volume/mute[/{address}].
func muteHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	msg, err := execVolumeMute(mux.Vars(r)["address"])
	if err != nil {
		appLog("volume", "POST /api/volume/mute failed: %v", err)
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, msg, nil)
}

// getActiveSourceHandler implements GET /api/source/active.
func getActiveSourceHandler(w http.ResponseWriter, r *http.Request) {
	c, ok := requireAdapter(func() {
		respondError(w, http.StatusServiceUnavailable, "CEC adapter not available")
	})
	if !ok {
		return
	}
	addr, err := c.GetActiveSource()
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, "Active source retrieved", map[string]interface{}{
		"address": int(addr),
		"name":    addr.String(),
	})
}

// setActiveSourceHandler implements POST /api/source/{address}.
func setActiveSourceHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	addr, err := strconv.Atoi(mux.Vars(r)["address"])
	if err != nil || addr < 0 || addr > 15 {
		respondError(w, http.StatusBadRequest, "Invalid logical address")
		return
	}
	if err := execSetActiveSource(addr); err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, fmt.Sprintf("Switched to device %d", addr), nil)
}

// setHDMIPortHandler implements POST /api/hdmi/{port}.
func setHDMIPortHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	port, err := strconv.Atoi(mux.Vars(r)["port"])
	if err != nil || port < 1 || port > 15 {
		respondError(w, http.StatusBadRequest, "Invalid HDMI port (must be 1-15)")
		return
	}
	if err := execHDMIPort(port); err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, fmt.Sprintf("Switched to HDMI port %d", port), nil)
}

// sendKeyHandler implements POST /api/key.
func sendKeyHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	var req struct {
		Address int    `json:"address"`
		Key     string `json:"key"`
		Keycode int    `json:"keycode"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "Invalid request body")
		return
	}
	if req.Address < 0 || req.Address > 15 {
		respondError(w, http.StatusBadRequest, "Invalid logical address")
		return
	}
	if req.Key == "" && req.Keycode == 0 {
		respondError(w, http.StatusBadRequest, "Either 'key' or 'keycode' must be provided (and keycode 0 must be specified via 'key': 'select')")
		return
	}
	appLog("nav", "POST /api/key addr=%d key=%q keycode=%d", req.Address, req.Key, req.Keycode)
	if err := execSendKey(req.Address, req.Key, req.Keycode); err != nil {
		appLog("nav", "POST /api/key failed: %v", err)
		if strings.Contains(err.Error(), "unsupported") || strings.Contains(err.Error(), "keycode") {
			respondError(w, http.StatusBadRequest, err.Error())
			return
		}
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, "Key command sent", nil)
}

// rawCommandHandler implements POST /api/command.
func rawCommandHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	var req struct {
		Initiator   int     `json:"initiator"`
		Destination int     `json:"destination"`
		Opcode      int     `json:"opcode"`
		Parameters  []uint8 `json:"parameters"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "Invalid request body")
		return
	}
	if req.Initiator < 0 || req.Initiator > 15 {
		respondError(w, http.StatusBadRequest, "Invalid initiator logical address (must be 0-15)")
		return
	}
	if req.Destination < 0 || req.Destination > 15 {
		respondError(w, http.StatusBadRequest, "Invalid destination logical address (must be 0-15)")
		return
	}
	if req.Opcode < 0 || req.Opcode > 0xFF {
		respondError(w, http.StatusBadRequest, "Invalid opcode (must be 0-255)")
		return
	}
	const maxCECParameters = 14
	if len(req.Parameters) > maxCECParameters {
		respondError(w, http.StatusBadRequest, fmt.Sprintf("Too many parameters (max %d)", maxCECParameters))
		return
	}

	cmd := &cec.Command{
		Initiator:   cec.LogicalAddress(req.Initiator),
		Destination: cec.LogicalAddress(req.Destination),
		Opcode:      cec.Opcode(req.Opcode),
		OpcodeSet:   true,
		Parameters:  req.Parameters,
	}

	appLog("cec", "POST /api/command initiator=%d destination=%d opcode=0x%02X params=%v",
		req.Initiator, req.Destination, req.Opcode, req.Parameters)
	err := adapter.With(func(c *cec.Connection) error { return c.Transmit(cmd) })
	if err != nil {
		appLog("cec", "POST /api/command transmit failed: %v", err)
		if err == ErrAdapterUnavailable {
			respondError(w, http.StatusServiceUnavailable, err.Error())
			return
		}
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	schedulePostCommandBusRefresh()
	respondSuccess(w, "Raw command sent", nil)
}

// getAudioStatusHandler implements GET /api/audio/status.
func getAudioStatusHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	displayVol, muted, raw, volumeRaw := execAudioStatusDisplay()
	respondSuccess(w, "Audio status retrieved", map[string]interface{}{
		"volume":     displayVol,
		"muted":      muted,
		"raw_status": raw,
		"volume_raw": volumeRaw,
	})
}

// getLogsHandler implements GET /api/logs.
func getLogsHandler(w http.ResponseWriter, r *http.Request) {
	respondSuccess(w, "Logs retrieved", logHandler.GetRecentLogs())
}

// healthHandler implements GET /api/health.
func healthHandler(w http.ResponseWriter, r *http.Request) {
	libInfo := ""
	ready := false
	if c := adapter.Conn(); c != nil {
		ready = true
		func() {
			defer func() {
				if recover() != nil {
					libInfo = "unavailable"
				}
			}()
			libInfo = c.GetLibInfo()
		}()
	}

	body := map[string]interface{}{
		"version":   version,
		"libcec":    libInfo,
		"cec_ready": ready,
	}
	if eventHub != nil {
		body["event_subscribers"] = eventHub.Subscribers()
		dropped, delivered := eventHub.Stats()
		body["events_delivered"] = delivered
		body["events_dropped"] = dropped
	}
	respondSuccess(w, "Service is healthy", body)
}
