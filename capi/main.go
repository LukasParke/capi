package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	"capi/cec"

	mqtt "github.com/eclipse/paho.mqtt.golang"
	"github.com/gorilla/mux"
)

// version is set at build time via -ldflags "-X main.version=..."
var version = "dev"

var (
	cecConn    *cec.Connection
	cecMutex   sync.Mutex
	cecReady   bool // true once CEC adapter is opened successfully
	logHandler *LogHandler
	eventHub   *EventHub
)

// CECEvent represents a real-time event from the CEC bus.
type CECEvent struct {
	Type      string      `json:"type"`      // "key_press", "command", "source_activated", "power_change", "alert"
	Timestamp time.Time   `json:"timestamp"`
	Data      interface{} `json:"data"`
}

// EventHub is a simple pub/sub hub for CEC events. Subscribers receive events on a channel.
type EventHub struct {
	mu          sync.RWMutex
	subs        map[chan CECEvent]struct{}
	bufferSize  int
}

// NewEventHub creates an event hub with the given subscriber channel buffer size.
func NewEventHub(bufferSize int) *EventHub {
	return &EventHub{
		subs:       make(map[chan CECEvent]struct{}),
		bufferSize: bufferSize,
	}
}

// Subscribe returns a channel that receives events. Caller must call Unsubscribe when done.
func (h *EventHub) Subscribe() chan CECEvent {
	ch := make(chan CECEvent, h.bufferSize)
	h.mu.Lock()
	h.subs[ch] = struct{}{}
	h.mu.Unlock()
	return ch
}

// Unsubscribe removes the channel from subscribers and closes it.
func (h *EventHub) Unsubscribe(ch chan CECEvent) {
	h.mu.Lock()
	delete(h.subs, ch)
	h.mu.Unlock()
	close(ch)
}

// Publish sends the event to all subscribers. Non-blocking: if a subscriber's channel is full, the event is dropped for that subscriber.
func (h *EventHub) Publish(ev CECEvent) {
	ev.Timestamp = time.Now()
	h.mu.RLock()
	defer h.mu.RUnlock()
	for ch := range h.subs {
		select {
		case ch <- ev:
		default:
			// subscriber slow or disconnected; drop
		}
	}
}

// LogHandler implements cec.CallbackHandler for logging
type LogHandler struct {
	LogMessages []LogMessage
	mu          sync.RWMutex
	maxMessages int
}

type LogMessage struct {
	Level     string    `json:"level"`
	Timestamp time.Time `json:"timestamp"`
	Message   string    `json:"message"`
}

func NewLogHandler() *LogHandler {
	return &LogHandler{
		LogMessages: make([]LogMessage, 0),
		maxMessages: 100,
	}
}

func (l *LogHandler) OnLogMessage(level cec.LogLevel, timestamp int64, message string) {
	l.mu.Lock()
	defer l.mu.Unlock()

	// libCEC log timestamps are provided as an int64 value. Treat this as
	// milliseconds since Unix epoch for conversion to time.Time.
	logTime := time.Unix(0, timestamp*int64(time.Millisecond))

	logMsg := LogMessage{
		Level:     level.String(),
		Timestamp: logTime,
		Message:   message,
	}

	l.LogMessages = append(l.LogMessages, logMsg)
	if len(l.LogMessages) > l.maxMessages {
		l.LogMessages = l.LogMessages[1:]
	}

	// Also log to console if not traffic
	if level != cec.LogLevelTraffic && level != cec.LogLevelDebug {
		log.Printf("[CEC %s] %s", level.String(), message)
	}
}

func (l *LogHandler) OnKeyPress(key cec.Keycode, duration uint32) {
	log.Printf("Key pressed: %d, duration: %d", key, duration)
	if eventHub != nil {
		eventHub.Publish(CECEvent{
			Type: "key_press",
			Data: map[string]interface{}{
				"keycode":  int(key),
				"duration": duration,
			},
		})
	}
}

func (l *LogHandler) OnCommand(command *cec.Command) {
	log.Printf("Command received: %s -> %s, opcode: 0x%02X",
		command.Initiator.String(), command.Destination.String(), command.Opcode)
	if eventHub != nil {
		data := map[string]interface{}{
			"initiator":   int(command.Initiator),
			"destination": int(command.Destination),
			"opcode":      fmt.Sprintf("0x%02X", command.Opcode),
		}
		// Emit power_change when we see ReportPowerStatus (initiator reports its status) or Standby
		if command.Opcode == cec.OpcodeReportPowerStatus && len(command.Parameters) >= 1 {
			eventHub.Publish(CECEvent{
				Type: "power_change",
				Data: map[string]interface{}{
					"address": int(command.Initiator),
					"status":  powerStatusFromByte(command.Parameters[0]),
				},
			})
		}
		if command.Opcode == cec.OpcodeStandby {
			eventHub.Publish(CECEvent{
				Type: "power_change",
				Data: map[string]interface{}{
					"address": int(command.Initiator),
					"status":  "standby",
				},
			})
		}
		eventHub.Publish(CECEvent{Type: "command", Data: data})
	}
}

func (l *LogHandler) OnConfigurationChanged(config *cec.Configuration) {
	log.Printf("Configuration changed: %s", config.DeviceName)
}

func (l *LogHandler) OnAlert(alert cec.Alert, param cec.Parameter) {
	log.Printf("Alert: %d", alert)
	if eventHub != nil {
		eventHub.Publish(CECEvent{
			Type: "alert",
			Data: map[string]interface{}{
				"alert": int(alert),
				"param": param.Value,
			},
		})
	}
}

func (l *LogHandler) OnMenuStateChanged(state cec.MenuState) bool {
	log.Printf("Menu state changed: %d", state)
	return true
}

func (l *LogHandler) OnSourceActivated(address cec.LogicalAddress, activated bool) {
	log.Printf("Source activated: %s, activated: %v", address.String(), activated)
	if eventHub != nil {
		eventHub.Publish(CECEvent{
			Type: "source_activated",
			Data: map[string]interface{}{
				"address":    int(address),
				"activated":  activated,
			},
		})
	}
}

// powerStatusFromByte maps CEC power status byte to string.
func powerStatusFromByte(b uint8) string {
	switch b {
	case 0x00:
		return "on"
	case 0x01:
		return "standby"
	case 0x02:
		return "transitioning_to_on"
	case 0x03:
		return "transitioning_to_standby"
	default:
		return "unknown"
	}
}

func (l *LogHandler) GetRecentLogs() []LogMessage {
	l.mu.RLock()
	defer l.mu.RUnlock()

	result := make([]LogMessage, len(l.LogMessages))
	copy(result, l.LogMessages)
	return result
}

// HTTP Handlers

type Response struct {
	Status  string      `json:"status"`
	Message string      `json:"message,omitempty"`
	Data    interface{} `json:"data,omitempty"`
}

func respondJSON(w http.ResponseWriter, status int, data interface{}) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(data)
}

func respondError(w http.ResponseWriter, status int, message string) {
	respondJSON(w, status, Response{
		Status:  "error",
		Message: message,
	})
}

func respondSuccess(w http.ResponseWriter, message string, data interface{}) {
	respondJSON(w, http.StatusOK, Response{
		Status:  "success",
		Message: message,
		Data:    data,
	})
}

// requireCEC checks whether the CEC adapter is available. If not, it sends a
// 503 response and returns false so the caller can bail out.
func requireCEC(w http.ResponseWriter) bool {
	cecMutex.Lock()
	ready := cecReady
	cecMutex.Unlock()
	if !ready {
		respondError(w, http.StatusServiceUnavailable, "CEC adapter not available")
		return false
	}
	return true
}

// Device endpoints

func deviceToMap(dev *cec.Device) map[string]interface{} {
	// Derive HDMI port from the first nibble of the physical address
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

func getDevicesHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	// Optionally force a rescan when requested by the client.
	rescanParam := r.URL.Query().Get("rescan")

	// Step 1: rescan (if requested) and get active address list — fast, hold lock briefly.
	cecMutex.Lock()
	if rescanParam == "1" || strings.EqualFold(rescanParam, "true") {
		cecConn.RescanDevices()
	}
	addresses := cecConn.GetActiveDevices()
	cecMutex.Unlock()

	// Step 2: query each device individually with a 20s overall deadline.
	// Each GetDeviceInfo call does several CEC queries that can be slow.
	deadline := time.After(20 * time.Second)
	result := make([]map[string]interface{}, 0, len(addresses))

	for _, addr := range addresses {
		select {
		case <-deadline:
			// Time's up — return what we have so far.
			respondSuccess(w, fmt.Sprintf("Devices retrieved (partial: %d of %d, CEC bus slow)", len(result), len(addresses)), result)
			return
		default:
		}

		cecMutex.Lock()
		dev, err := cecConn.GetDeviceInfo(addr)
		cecMutex.Unlock()

		if err == nil {
			result = append(result, deviceToMap(dev))
		}
	}

	respondSuccess(w, "Devices retrieved", result)
}

func getDeviceHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	vars := mux.Vars(r)
	addrStr := vars["address"]

	addr, err := strconv.Atoi(addrStr)
	if err != nil || addr < 0 || addr > 15 {
		respondError(w, http.StatusBadRequest, "Invalid logical address")
		return
	}

	cecMutex.Lock()
	defer cecMutex.Unlock()

	device, err := cecConn.GetDeviceInfo(cec.LogicalAddress(addr))
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	result := map[string]interface{}{
		"logical_address":  int(device.LogicalAddress),
		"address_name":     device.LogicalAddress.String(),
		"physical_address": cec.PhysicalAddressToString(device.PhysicalAddress),
		"vendor_id":        fmt.Sprintf("0x%06X", device.VendorID),
		"vendor_name":      cec.GetVendorName(device.VendorID),
		"cec_version":      device.CECVersion.String(),
		"power_status":     device.PowerStatus.String(),
		"osd_name":         device.OSDName,
		"menu_language":    device.MenuLanguage,
		"is_active":        device.IsActive,
		"is_active_source": device.IsActiveSource,
	}

	respondSuccess(w, "Device info retrieved", result)
}

// Power control endpoints

func powerOnHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	vars := mux.Vars(r)
	addrStr := vars["address"]

	addr := 0 // TV by default
	if addrStr != "" {
		var err error
		addr, err = strconv.Atoi(addrStr)
		if err != nil || addr < 0 || addr > 15 {
			respondError(w, http.StatusBadRequest, "Invalid logical address")
			return
		}
	}

	cecMutex.Lock()
	defer cecMutex.Unlock()

	err := cecConn.PowerOn(cec.LogicalAddress(addr))
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, fmt.Sprintf("Power on command sent to device %d", addr), nil)
}

func powerOffHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	vars := mux.Vars(r)
	addrStr := vars["address"]

	addr := 0 // TV by default
	if addrStr != "" {
		var err error
		addr, err = strconv.Atoi(addrStr)
		if err != nil || addr < 0 || addr > 15 {
			respondError(w, http.StatusBadRequest, "Invalid logical address")
			return
		}
	}

	cecMutex.Lock()
	defer cecMutex.Unlock()

	err := cecConn.Standby(cec.LogicalAddress(addr))
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, fmt.Sprintf("Standby command sent to device %d", addr), nil)
}

func getPowerStatusHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	vars := mux.Vars(r)
	addrStr := vars["address"]

	addr := 0 // TV by default
	if addrStr != "" {
		var err error
		addr, err = strconv.Atoi(addrStr)
		if err != nil || addr < 0 || addr > 15 {
			respondError(w, http.StatusBadRequest, "Invalid logical address")
			return
		}
	}

	cecMutex.Lock()
	defer cecMutex.Unlock()

	status, err := cecConn.GetDevicePowerStatus(cec.LogicalAddress(addr))
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, "Power status retrieved", map[string]interface{}{
		"address": addr,
		"status":  status.String(),
	})
}

// Volume control endpoints

func volumeUpHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	vars := mux.Vars(r)
	addrStr := vars["address"]

	cecMutex.Lock()
	defer cecMutex.Unlock()

	if addrStr != "" {
		// Send volume key directly to a specific device
		addr, err := strconv.Atoi(addrStr)
		if err != nil || addr < 0 || addr > 15 {
			respondError(w, http.StatusBadRequest, "invalid address")
			return
		}
		err = cecConn.SendVolumeKey(cec.LogicalAddress(addr), cec.KeycodeVolumeUp)
		if err != nil {
			respondError(w, http.StatusInternalServerError, err.Error())
			return
		}
		respondSuccess(w, fmt.Sprintf("Volume up sent to device %d", addr), nil)
		return
	}

	// Default: send to audio system via libcec
	err := cecConn.VolumeUp(true)
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, "Volume up command sent", nil)
}

func volumeDownHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	vars := mux.Vars(r)
	addrStr := vars["address"]

	cecMutex.Lock()
	defer cecMutex.Unlock()

	if addrStr != "" {
		addr, err := strconv.Atoi(addrStr)
		if err != nil || addr < 0 || addr > 15 {
			respondError(w, http.StatusBadRequest, "invalid address")
			return
		}
		err = cecConn.SendVolumeKey(cec.LogicalAddress(addr), cec.KeycodeVolumeDown)
		if err != nil {
			respondError(w, http.StatusInternalServerError, err.Error())
			return
		}
		respondSuccess(w, fmt.Sprintf("Volume down sent to device %d", addr), nil)
		return
	}

	err := cecConn.VolumeDown(true)
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, "Volume down command sent", nil)
}

func muteHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	vars := mux.Vars(r)
	addrStr := vars["address"]

	cecMutex.Lock()
	defer cecMutex.Unlock()

	if addrStr != "" {
		addr, err := strconv.Atoi(addrStr)
		if err != nil || addr < 0 || addr > 15 {
			respondError(w, http.StatusBadRequest, "invalid address")
			return
		}
		err = cecConn.SendVolumeKey(cec.LogicalAddress(addr), cec.KeycodeMute)
		if err != nil {
			respondError(w, http.StatusInternalServerError, err.Error())
			return
		}
		respondSuccess(w, fmt.Sprintf("Mute sent to device %d", addr), nil)
		return
	}

	err := cecConn.AudioToggleMute()
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, "Mute toggle command sent", nil)
}

// Source control endpoints

func getActiveSourceHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	cecMutex.Lock()
	defer cecMutex.Unlock()

	addr, err := cecConn.GetActiveSource()
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, "Active source retrieved", map[string]interface{}{
		"address": int(addr),
		"name":    addr.String(),
	})
}

func setActiveSourceHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	vars := mux.Vars(r)
	addrStr := vars["address"]

	addr, err := strconv.Atoi(addrStr)
	if err != nil || addr < 0 || addr > 15 {
		respondError(w, http.StatusBadRequest, "Invalid logical address")
		return
	}

	cecMutex.Lock()
	defer cecMutex.Unlock()

	err = cecConn.SwitchToDevice(cec.LogicalAddress(addr))
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, fmt.Sprintf("Switched to device %d", addr), nil)
}

func setHDMIPortHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	vars := mux.Vars(r)
	portStr := vars["port"]

	port, err := strconv.Atoi(portStr)
	if err != nil || port < 1 || port > 15 {
		respondError(w, http.StatusBadRequest, "Invalid HDMI port (must be 1-15)")
		return
	}

	cecMutex.Lock()
	defer cecMutex.Unlock()

	err = cecConn.SwitchToHDMIPort(uint8(port))
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, fmt.Sprintf("Switched to HDMI port %d", port), nil)
}

// Navigation endpoints

func sendKeyHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
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

	// Require that either a known key name or an explicit keycode is provided.
	if req.Key == "" && req.Keycode == 0 {
		respondError(w, http.StatusBadRequest, "Either 'key' or 'keycode' must be provided (and keycode 0 must be specified via 'key': 'select')")
		return
	}

	var keycode cec.Keycode

	// Map string keys to keycodes if provided
	if req.Key != "" {
		keyMap := map[string]cec.Keycode{
			"up":     cec.KeycodeUp,
			"down":   cec.KeycodeDown,
			"left":   cec.KeycodeLeft,
			"right":  cec.KeycodeRight,
			"select": cec.KeycodeSelect,
			"enter":  cec.KeycodeEnter,
			"back":   cec.KeycodeExit,
			"home":   cec.KeycodeRootMenu,
			"menu":   cec.KeycodeSetupMenu,
			"play":   cec.KeycodePlay,
			"pause":  cec.KeycodePause,
			"stop":   cec.KeycodeStop,
		}
		if k, ok := keyMap[req.Key]; ok {
			keycode = k
		} else {
			respondError(w, http.StatusBadRequest, "Unsupported key name")
			return
		}
	} else {
		// No key string; validate raw keycode range explicitly.
		if req.Keycode < 0 || req.Keycode > 0xFF {
			respondError(w, http.StatusBadRequest, "Keycode must be in range 0-255")
			return
		}
		keycode = cec.Keycode(req.Keycode)
	}

	cecMutex.Lock()
	defer cecMutex.Unlock()

	err := cecConn.SendButton(cec.LogicalAddress(req.Address), keycode)
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, "Key command sent", nil)
}

// Raw command endpoint

func rawCommandHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
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

	// Validate logical addresses
	if req.Initiator < 0 || req.Initiator > 15 {
		respondError(w, http.StatusBadRequest, "Invalid initiator logical address (must be 0-15)")
		return
	}
	if req.Destination < 0 || req.Destination > 15 {
		respondError(w, http.StatusBadRequest, "Invalid destination logical address (must be 0-15)")
		return
	}

	// Validate opcode
	if req.Opcode < 0 || req.Opcode > 0xFF {
		respondError(w, http.StatusBadRequest, "Invalid opcode (must be 0-255)")
		return
	}

	// Conservative limit on parameter bytes for a single CEC frame.
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

	cecMutex.Lock()
	defer cecMutex.Unlock()

	err := cecConn.Transmit(cmd)
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, "Raw command sent", nil)
}

// Logs endpoint

func getLogsHandler(w http.ResponseWriter, r *http.Request) {
	logs := logHandler.GetRecentLogs()
	respondSuccess(w, "Logs retrieved", logs)
}

// SSE endpoint: GET /api/events streams CEC events as Server-Sent Events.
func eventsSSEHandler(w http.ResponseWriter, r *http.Request) {
	if eventHub == nil {
		respondError(w, http.StatusInternalServerError, "event hub not initialized")
		return
	}

	flusher, ok := w.(http.Flusher)
	if !ok {
		respondError(w, http.StatusInternalServerError, "streaming unsupported")
		return
	}

	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	w.WriteHeader(http.StatusOK)
	flusher.Flush()

	ch := eventHub.Subscribe()
	defer eventHub.Unsubscribe(ch)

	// Send keepalive comment every 15s so proxies don't close the connection
	ticker := time.NewTicker(15 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case ev, ok := <-ch:
			if !ok {
				return
			}
			body, err := json.Marshal(ev)
			if err != nil {
				continue
			}
			fmt.Fprintf(w, "data: %s\n\n", body)
			flusher.Flush()
		case <-ticker.C:
			fmt.Fprintf(w, ": keepalive\n\n")
			flusher.Flush()
		case <-r.Context().Done():
			return
		}
	}
}

// Topology endpoint

func getTopologyHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	cecMutex.Lock()
	topo := cecConn.GetBusTopology()
	ownAddrs := cecConn.GetLogicalAddresses()
	cecMutex.Unlock()

	// Build port list with device names for the UI
	type portDetail struct {
		Port    int      `json:"port"`
		Devices []string `json:"devices"`
	}
	ports := make([]portDetail, 0, len(topo.ActivePorts))
	for _, p := range topo.ActivePorts {
		names := make([]string, 0, len(p.Devices))
		for _, addr := range p.Devices {
			cecMutex.Lock()
			name, _ := cecConn.GetDeviceOSDName(addr)
			cecMutex.Unlock()
			if name == "" {
				name = addr.String()
			}
			names = append(names, name)
		}
		ports = append(ports, portDetail{Port: int(p.Port), Devices: names})
	}

	ownAddrInts := make([]int, len(ownAddrs))
	for i, a := range ownAddrs {
		ownAddrInts[i] = int(a)
	}

	respondSuccess(w, "Bus topology retrieved", map[string]interface{}{
		"own_addresses":    ownAddrInts,
		"own_port":         int(topo.OwnPort),
		"known_port_count": int(topo.KnownPortCount),
		"active_ports":     ports,
	})
}

// Audio status endpoint

func getAudioStatusHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) { return }
	cecMutex.Lock()
	volume, muted, err := cecConn.GetAudioStatus()
	cecMutex.Unlock()

	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, "Audio status retrieved", map[string]interface{}{
		"volume": int(volume),
		"muted":  muted,
	})
}

// ── Configuration persistence ──────────────────────────────────────────

// MQTTConfig holds MQTT broker connection settings.
type MQTTConfig struct {
	Broker string `json:"broker"`
	User   string `json:"user"`
	Pass   string `json:"pass"`
	Prefix string `json:"prefix"`
}

// Config is the on-disk configuration file format.
type Config struct {
	MQTT MQTTConfig `json:"mqtt"`
}

var (
	currentConfig  Config
	configMu       sync.RWMutex
	configFilePath string
)

// loadConfig reads and parses the config file. Returns zero Config if not found.
func loadConfig(path string) Config {
	var cfg Config
	data, err := os.ReadFile(path)
	if err != nil {
		return cfg
	}
	_ = json.Unmarshal(data, &cfg)
	return cfg
}

// saveConfig atomically writes the config file.
func saveConfig(path string, cfg Config) error {
	data, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return err
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0600); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// ── MQTT bridge ────────────────────────────────────────────────────────

var (
	mqttClient mqtt.Client
	mqttMu     sync.Mutex
	mqttCancel context.CancelFunc
)

// stopMQTT disconnects the MQTT client and cancels the event-forwarding goroutine.
func stopMQTT() {
	mqttMu.Lock()
	defer mqttMu.Unlock()
	if mqttCancel != nil {
		mqttCancel()
		mqttCancel = nil
	}
	if mqttClient != nil && mqttClient.IsConnected() {
		mqttClient.Disconnect(1000)
		log.Println("[MQTT] Disconnected")
	}
	mqttClient = nil
}

// startMQTT connects to the broker, subscribes to command topics, and
// forwards EventHub events to MQTT publish topics. Safe to call multiple
// times; previous connections are torn down first.
func startMQTT(broker, user, pass, prefix string) {
	stopMQTT()

	host, _ := os.Hostname()
	opts := mqtt.NewClientOptions().
		AddBroker(broker).
		SetClientID(fmt.Sprintf("capi-%s-%d", host, os.Getpid())).
		SetAutoReconnect(true).
		SetConnectRetry(true).
		SetConnectRetryInterval(10 * time.Second).
		SetOnConnectHandler(func(c mqtt.Client) {
			log.Printf("[MQTT] Connected to %s", broker)
			cmdTopic := prefix + "/command/#"
			token := c.Subscribe(cmdTopic, 1, func(_ mqtt.Client, msg mqtt.Message) {
				handleMQTTCommand(prefix, msg.Topic(), msg.Payload())
			})
			if token.Wait() && token.Error() != nil {
				log.Printf("[MQTT] Subscribe failed: %v", token.Error())
			} else {
				log.Printf("[MQTT] Subscribed to %s", cmdTopic)
			}
		}).
		SetConnectionLostHandler(func(_ mqtt.Client, err error) {
			log.Printf("[MQTT] Connection lost: %v", err)
		})

	if user != "" {
		opts.SetUsername(user)
	}
	if pass != "" {
		opts.SetPassword(pass)
	}

	ctx, cancel := context.WithCancel(context.Background())

	mqttMu.Lock()
	mqttCancel = cancel
	mqttClient = mqtt.NewClient(opts)
	client := mqttClient
	mqttMu.Unlock()

	if token := client.Connect(); token.Wait() && token.Error() != nil {
		log.Printf("[MQTT] Initial connection failed (will retry): %v", token.Error())
	}

	// Goroutine: forward EventHub events to MQTT
	go func() {
		ch := eventHub.Subscribe()
		defer eventHub.Unsubscribe(ch)
		for {
			select {
			case <-ctx.Done():
				return
			case ev, ok := <-ch:
				if !ok {
					return
				}
				mqttMu.Lock()
				c := mqttClient
				mqttMu.Unlock()
				if c == nil || !c.IsConnected() {
					continue
				}
				topic := prefix + "/event/" + ev.Type
				payload, err := json.Marshal(ev.Data)
				if err != nil {
					continue
				}
				c.Publish(topic, 0, false, payload)
			}
		}
	}()
}

// handleMQTTCommand dispatches an incoming MQTT message to the appropriate
// CEC operation. Topic format: {prefix}/command/{action}[/{param}]
func handleMQTTCommand(prefix, topic string, payload []byte) {
	cecMutex.Lock()
	ready := cecReady
	cecMutex.Unlock()
	if !ready {
		log.Printf("[MQTT] Ignoring command %q: CEC adapter not available", topic)
		return
	}

	cmdPath := strings.TrimPrefix(topic, prefix+"/command/")

	switch {
	case cmdPath == "power/on":
		addr := parseMQTTAddress(payload, 0)
		if addr < 0 || addr > 15 {
			log.Printf("[MQTT] power/on: invalid address %q", string(payload))
			return
		}
		cecMutex.Lock()
		err := cecConn.PowerOn(cec.LogicalAddress(addr))
		cecMutex.Unlock()
		if err != nil {
			log.Printf("[MQTT] power/on failed: %v", err)
		}

	case cmdPath == "power/off":
		addr := parseMQTTAddress(payload, 0)
		if addr < 0 || addr > 15 {
			log.Printf("[MQTT] power/off: invalid address %q", string(payload))
			return
		}
		cecMutex.Lock()
		err := cecConn.Standby(cec.LogicalAddress(addr))
		cecMutex.Unlock()
		if err != nil {
			log.Printf("[MQTT] power/off failed: %v", err)
		}

	case cmdPath == "volume/up":
		cecMutex.Lock()
		err := cecConn.VolumeUp(true)
		cecMutex.Unlock()
		if err != nil {
			log.Printf("[MQTT] volume/up failed: %v", err)
		}

	case cmdPath == "volume/down":
		cecMutex.Lock()
		err := cecConn.VolumeDown(true)
		cecMutex.Unlock()
		if err != nil {
			log.Printf("[MQTT] volume/down failed: %v", err)
		}

	case cmdPath == "volume/mute":
		cecMutex.Lock()
		err := cecConn.AudioToggleMute()
		cecMutex.Unlock()
		if err != nil {
			log.Printf("[MQTT] volume/mute failed: %v", err)
		}

	case cmdPath == "source":
		addr := parseMQTTAddress(payload, -1)
		if addr < 0 || addr > 15 {
			log.Printf("[MQTT] source: invalid address %q", string(payload))
			return
		}
		cecMutex.Lock()
		err := cecConn.SwitchToDevice(cec.LogicalAddress(addr))
		cecMutex.Unlock()
		if err != nil {
			log.Printf("[MQTT] source failed: %v", err)
		}

	case cmdPath == "hdmi":
		port := parseMQTTAddress(payload, -1)
		if port < 1 || port > 15 {
			log.Printf("[MQTT] hdmi: invalid port %q", string(payload))
			return
		}
		cecMutex.Lock()
		err := cecConn.SwitchToHDMIPort(uint8(port))
		cecMutex.Unlock()
		if err != nil {
			log.Printf("[MQTT] hdmi failed: %v", err)
		}

	case cmdPath == "key":
		var req struct {
			Address int    `json:"address"`
			Key     string `json:"key"`
			Keycode int    `json:"keycode"`
		}
		if err := json.Unmarshal(payload, &req); err != nil {
			log.Printf("[MQTT] key: invalid payload: %v", err)
			return
		}
		if req.Address < 0 || req.Address > 15 {
			log.Printf("[MQTT] key: invalid address %d", req.Address)
			return
		}
		keyMap := map[string]cec.Keycode{
			"up": cec.KeycodeUp, "down": cec.KeycodeDown,
			"left": cec.KeycodeLeft, "right": cec.KeycodeRight,
			"select": cec.KeycodeSelect, "enter": cec.KeycodeEnter,
			"back": cec.KeycodeExit, "home": cec.KeycodeRootMenu,
			"menu": cec.KeycodeSetupMenu, "play": cec.KeycodePlay,
			"pause": cec.KeycodePause, "stop": cec.KeycodeStop,
		}
		var keycode cec.Keycode
		if req.Key != "" {
			k, ok := keyMap[req.Key]
			if !ok {
				log.Printf("[MQTT] key: unknown key name %q", req.Key)
				return
			}
			keycode = k
		} else {
			keycode = cec.Keycode(req.Keycode)
		}
		cecMutex.Lock()
		err := cecConn.SendButton(cec.LogicalAddress(req.Address), keycode)
		cecMutex.Unlock()
		if err != nil {
			log.Printf("[MQTT] key failed: %v", err)
		}

	default:
		log.Printf("[MQTT] Unknown command topic: %s", topic)
	}
}

// parseMQTTAddress parses a simple integer from the payload (trimmed).
// Returns defaultVal if the payload is empty or not a valid integer.
func parseMQTTAddress(payload []byte, defaultVal int) int {
	s := strings.TrimSpace(string(payload))
	if s == "" {
		return defaultVal
	}
	v, err := strconv.Atoi(s)
	if err != nil {
		return defaultVal
	}
	return v
}

// ── MQTT settings API ──────────────────────────────────────────────────

func getMQTTSettingsHandler(w http.ResponseWriter, r *http.Request) {
	configMu.RLock()
	cfg := currentConfig.MQTT
	configMu.RUnlock()

	maskedPass := ""
	if cfg.Pass != "" {
		maskedPass = "***"
	}

	mqttMu.Lock()
	connected := mqttClient != nil && mqttClient.IsConnected()
	mqttMu.Unlock()

	respondSuccess(w, "MQTT settings", map[string]interface{}{
		"broker":    cfg.Broker,
		"user":      cfg.User,
		"pass":      maskedPass,
		"prefix":    cfg.Prefix,
		"connected": connected,
	})
}

func postMQTTSettingsHandler(w http.ResponseWriter, r *http.Request) {
	var req struct {
		Broker string `json:"broker"`
		User   string `json:"user"`
		Pass   string `json:"pass"`
		Prefix string `json:"prefix"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "Invalid request body")
		return
	}
	if req.Prefix == "" {
		req.Prefix = "capi"
	}

	configMu.Lock()
	// Sentinel "***" means keep existing password
	if req.Pass == "***" {
		req.Pass = currentConfig.MQTT.Pass
	}
	currentConfig.MQTT = MQTTConfig{
		Broker: req.Broker,
		User:   req.User,
		Pass:   req.Pass,
		Prefix: req.Prefix,
	}
	cfg := currentConfig
	configMu.Unlock()

	if err := saveConfig(configFilePath, cfg); err != nil {
		log.Printf("Failed to save config: %v", err)
		respondError(w, http.StatusInternalServerError, fmt.Sprintf("Failed to save config: %v", err))
		return
	}

	if req.Broker != "" {
		startMQTT(req.Broker, req.User, req.Pass, req.Prefix)
	} else {
		stopMQTT()
	}

	respondSuccess(w, "MQTT settings saved", nil)
}

// ── Self-update logic ──────────────────────────────────────────────────

const updateRepo = "LukasParke/capi"

var updateHTTPClient = &http.Client{Timeout: 30 * time.Second}

// releaseInfo holds metadata about a GitHub release.
type releaseInfo struct {
	TagName string        `json:"tag_name"`
	Assets  []releaseAsset `json:"assets"`
}

type releaseAsset struct {
	Name               string `json:"name"`
	BrowserDownloadURL string `json:"browser_download_url"`
}

// checkForUpdate queries the GitHub releases API and returns info about the
// latest release. Returns nil if the current version is already up to date.
func checkForUpdate() (*releaseInfo, error) {
	url := fmt.Sprintf("https://api.github.com/repos/%s/releases/latest", updateRepo)
	resp, err := updateHTTPClient.Get(url)
	if err != nil {
		return nil, fmt.Errorf("failed to query GitHub: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("GitHub API returned %d", resp.StatusCode)
	}

	var info releaseInfo
	if err := json.NewDecoder(resp.Body).Decode(&info); err != nil {
		return nil, fmt.Errorf("failed to parse release JSON: %w", err)
	}

	if info.TagName == version {
		return nil, nil // already up to date
	}

	return &info, nil
}

// assetURL finds the download URL for the named asset in a release.
func assetURL(info *releaseInfo, name string) string {
	for _, a := range info.Assets {
		if a.Name == name {
			return a.BrowserDownloadURL
		}
	}
	return ""
}

// binaryAssetName returns the release asset name for the current architecture.
func binaryAssetName() string {
	switch runtime.GOARCH {
	case "arm64":
		return "capi-linux-arm64"
	case "arm":
		return "capi-linux-armv6"
	default:
		return "capi-linux-arm64"
	}
}

// downloadFile downloads a URL to a local file path.
func downloadFile(url, dest string) error {
	resp, err := updateHTTPClient.Get(url)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("download returned %d", resp.StatusCode)
	}

	tmp := dest + ".tmp"
	f, err := os.Create(tmp)
	if err != nil {
		return err
	}

	if _, err := io.Copy(f, resp.Body); err != nil {
		f.Close()
		os.Remove(tmp)
		return err
	}
	f.Close()

	if err := os.Chmod(tmp, 0755); err != nil {
		os.Remove(tmp)
		return err
	}

	return os.Rename(tmp, dest)
}

// performUpdate downloads the new binary and index.html from the given release.
func performUpdate(info *releaseInfo) error {
	binName := binaryAssetName()
	binURL := assetURL(info, binName)
	if binURL == "" {
		return fmt.Errorf("release %s has no asset %s", info.TagName, binName)
	}

	exe, err := os.Executable()
	if err != nil {
		exe = "/opt/capi/capi"
	}
	installDir := filepath.Dir(exe)

	log.Printf("Downloading %s from %s ...", binName, info.TagName)
	if err := downloadFile(binURL, filepath.Join(installDir, "capi")); err != nil {
		return fmt.Errorf("binary download failed: %w", err)
	}

	// Also update index.html if present in release assets
	htmlURL := assetURL(info, "index.html")
	if htmlURL != "" {
		log.Println("Downloading updated index.html ...")
		if err := downloadFile(htmlURL, filepath.Join(installDir, "index.html")); err != nil {
			log.Printf("Warning: index.html download failed: %v", err)
		}
	}

	return nil
}

// restartService asks systemd to restart the capi service.
func restartService() error {
	cmd := exec.Command("systemctl", "restart", "capi.service")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

// doSelfUpdate is the CLI entry-point for `capi -update`.
func doSelfUpdate() {
	log.Printf("Current version: %s", version)
	log.Println("Checking for updates...")

	info, err := checkForUpdate()
	if err != nil {
		log.Fatalf("Update check failed: %v", err)
	}
	if info == nil {
		log.Println("Already up to date.")
		os.Exit(0)
	}

	log.Printf("Update available: %s -> %s", version, info.TagName)

	if err := performUpdate(info); err != nil {
		log.Fatalf("Update failed: %v", err)
	}

	log.Println("Update downloaded. Restarting service...")
	if err := restartService(); err != nil {
		log.Printf("Could not restart service: %v (you may need to restart manually)", err)
	}
	os.Exit(0)
}

// POST /api/update handler

func updateHandler(w http.ResponseWriter, r *http.Request) {
	info, err := checkForUpdate()
	if err != nil {
		respondError(w, http.StatusBadGateway, fmt.Sprintf("Update check failed: %v", err))
		return
	}
	if info == nil {
		respondSuccess(w, "Already up to date", map[string]interface{}{
			"version": version,
		})
		return
	}

	if err := performUpdate(info); err != nil {
		respondError(w, http.StatusInternalServerError, fmt.Sprintf("Update failed: %v", err))
		return
	}

	respondSuccess(w, fmt.Sprintf("Updated to %s, restarting...", info.TagName), map[string]interface{}{
		"old_version": version,
		"new_version": info.TagName,
	})

	// Restart after a short delay so the HTTP response is sent first
	go func() {
		time.Sleep(1 * time.Second)
		restartService()
	}()
}

// Health check

func healthHandler(w http.ResponseWriter, r *http.Request) {
	cecMutex.Lock()
	ready := cecReady
	libInfo := ""
	if cecConn != nil {
		// Protect against any unexpected panics inside libcec.
		func() {
			defer func() {
				if recover() != nil {
					libInfo = "unavailable"
				}
			}()
			libInfo = cecConn.GetLibInfo()
		}()
	}
	cecMutex.Unlock()

	respondSuccess(w, "Service is healthy", map[string]interface{}{
		"version":   version,
		"libcec":    libInfo,
		"cec_ready": ready,
	})
}

func main() {
	bindAddr := flag.String("bind", ":8080", "Bind address (e.g., :8080 for all interfaces, localhost:8080 for local only)")
	deviceName := flag.String("name", "CEC HTTP Bridge", "Device name")
	adapterPath := flag.String("adapter", "", "CEC adapter path (auto-detect if empty)")
	showVersion := flag.Bool("version", false, "Print version and exit")
	doUpdate := flag.Bool("update", false, "Check for updates and install the latest release")
	mqttBroker := flag.String("mqtt-broker", "", "MQTT broker URL (e.g. tcp://localhost:1883). Empty disables MQTT.")
	mqttUser := flag.String("mqtt-user", "", "MQTT username (optional)")
	mqttPass := flag.String("mqtt-pass", "", "MQTT password (optional)")
	mqttPrefix := flag.String("mqtt-prefix", "capi", "MQTT topic prefix")
	flag.Parse()

	if *showVersion {
		fmt.Println(version)
		os.Exit(0)
	}

	if *doUpdate {
		doSelfUpdate()
		return
	}

	// Determine config file path (next to the binary)
	exe, _ := os.Executable()
	configFilePath = filepath.Join(filepath.Dir(exe), "config.json")

	// Load persisted config; CLI flags override config file values
	currentConfig = loadConfig(configFilePath)
	if *mqttBroker != "" {
		currentConfig.MQTT.Broker = *mqttBroker
	}
	if *mqttUser != "" {
		currentConfig.MQTT.User = *mqttUser
	}
	if *mqttPass != "" {
		currentConfig.MQTT.Pass = *mqttPass
	}
	flag.Visit(func(f *flag.Flag) {
		if f.Name == "mqtt-prefix" {
			currentConfig.MQTT.Prefix = *mqttPrefix
		}
	})
	if currentConfig.MQTT.Prefix == "" {
		currentConfig.MQTT.Prefix = "capi"
	}

	// Set up event hub and logging (independent of CEC)
	eventHub = NewEventHub(64)
	logHandler = NewLogHandler()

	// Initialize CEC in background so the HTTP server starts regardless
	go func() {
		const maxBackoff = 60 * time.Second
		backoff := 3 * time.Second

		for {
			log.Println("Initializing CEC connection...")
			conn, err := cec.Open(*deviceName, cec.DeviceTypeRecordingDevice)
			if err != nil {
				log.Printf("Failed to initialize CEC: %v — retrying in %v", err, backoff)
				time.Sleep(backoff)
				if backoff < maxBackoff {
					backoff *= 2
					if backoff > maxBackoff {
						backoff = maxBackoff
					}
				}
				continue
			}

			conn.SetCallbackHandler(logHandler)

			// Find adapter
			adapter := *adapterPath
			if adapter == "" {
				log.Println("Searching for CEC adapters...")
				adapters, err := conn.FindAdapters()
				if err != nil || len(adapters) == 0 {
					log.Printf("No CEC adapters found — retrying in %v", backoff)
					conn.Close()
					time.Sleep(backoff)
					if backoff < maxBackoff {
						backoff *= 2
						if backoff > maxBackoff {
							backoff = maxBackoff
						}
					}
					continue
				}
				if adapters[0].Comm != "" && strings.HasPrefix(adapters[0].Comm, "/dev/") {
					adapter = adapters[0].Comm
				} else {
					adapter = adapters[0].Path
				}
				log.Printf("Found adapter: %s", adapter)
			}

			// Open adapter
			log.Printf("Opening CEC adapter: %s", adapter)
			if err := conn.OpenAdapter(adapter); err != nil {
				log.Printf("Failed to open CEC adapter: %v — retrying in %v", err, backoff)
				conn.Close()
				time.Sleep(backoff)
				if backoff < maxBackoff {
					backoff *= 2
					if backoff > maxBackoff {
						backoff = maxBackoff
					}
				}
				continue
			}

			log.Println("CEC connection established")
			log.Println(conn.GetLibInfo())

			// Wait for CEC bus to settle
			time.Sleep(2 * time.Second)

			// Publish the connection
			cecMutex.Lock()
			cecConn = conn
			cecReady = true
			cecMutex.Unlock()

			log.Println("CEC adapter is ready")

			// Start MQTT bridge if configured
			if currentConfig.MQTT.Broker != "" {
				startMQTT(currentConfig.MQTT.Broker, currentConfig.MQTT.User, currentConfig.MQTT.Pass, currentConfig.MQTT.Prefix)
			}
			return
		}
	}()

	// Set up HTTP router
	r := mux.NewRouter()

	// Web UI — load index.html from same directory as the binary
	r.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		exe, _ := os.Executable()
		htmlPath := filepath.Join(filepath.Dir(exe), "index.html")
		data, err := os.ReadFile(htmlPath)
		if err != nil {
			http.Error(w, "UI not found", http.StatusNotFound)
			return
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		w.Write(data)
	}).Methods("GET")

	// Device endpoints
	r.HandleFunc("/api/devices", getDevicesHandler).Methods("GET")
	r.HandleFunc("/api/devices/{address}", getDeviceHandler).Methods("GET")

	// Power control
	r.HandleFunc("/api/power/on", powerOnHandler).Methods("POST")
	r.HandleFunc("/api/power/on/{address}", powerOnHandler).Methods("POST")
	r.HandleFunc("/api/power/off", powerOffHandler).Methods("POST")
	r.HandleFunc("/api/power/off/{address}", powerOffHandler).Methods("POST")
	r.HandleFunc("/api/power/status", getPowerStatusHandler).Methods("GET")
	r.HandleFunc("/api/power/status/{address}", getPowerStatusHandler).Methods("GET")

	// Volume control
	r.HandleFunc("/api/volume/up", volumeUpHandler).Methods("POST")
	r.HandleFunc("/api/volume/up/{address}", volumeUpHandler).Methods("POST")
	r.HandleFunc("/api/volume/down", volumeDownHandler).Methods("POST")
	r.HandleFunc("/api/volume/down/{address}", volumeDownHandler).Methods("POST")
	r.HandleFunc("/api/volume/mute", muteHandler).Methods("POST")
	r.HandleFunc("/api/volume/mute/{address}", muteHandler).Methods("POST")

	// Source control
	r.HandleFunc("/api/source/active", getActiveSourceHandler).Methods("GET")
	r.HandleFunc("/api/source/{address}", setActiveSourceHandler).Methods("POST")
	r.HandleFunc("/api/hdmi/{port}", setHDMIPortHandler).Methods("POST")

	// Topology
	r.HandleFunc("/api/topology", getTopologyHandler).Methods("GET")

	// Audio status
	r.HandleFunc("/api/audio/status", getAudioStatusHandler).Methods("GET")

	// Navigation
	r.HandleFunc("/api/key", sendKeyHandler).Methods("POST")

	// Raw command
	r.HandleFunc("/api/command", rawCommandHandler).Methods("POST")

	// Logs
	r.HandleFunc("/api/logs", getLogsHandler).Methods("GET")

	// Server-Sent Events (real-time CEC bus events)
	r.HandleFunc("/api/events", eventsSSEHandler).Methods("GET")

	// Health
	r.HandleFunc("/api/health", healthHandler).Methods("GET")

	// Self-update
	r.HandleFunc("/api/update", updateHandler).Methods("POST")

	// MQTT settings
	r.HandleFunc("/api/settings/mqtt", getMQTTSettingsHandler).Methods("GET")
	r.HandleFunc("/api/settings/mqtt", postMQTTSettingsHandler).Methods("POST")

	// Start server with graceful shutdown (signal.Notify works on Go 1.15+)
	server := &http.Server{Addr: *bindAddr, Handler: r}
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM)

	go func() {
		log.Printf("Starting HTTP server on %s", *bindAddr)
		log.Printf("API documentation: http://%s/api/health", *bindAddr)
		if err := server.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			log.Fatalf("Server failed: %v", err)
		}
	}()

	<-sigChan
	log.Println("Shutting down...")
	stopMQTT()
	shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if err := server.Shutdown(shutdownCtx); err != nil {
		log.Printf("HTTP server shutdown: %v", err)
	}
	// Close CEC connection if it was established
	cecMutex.Lock()
	if cecConn != nil {
		cecConn.Close()
	}
	cecMutex.Unlock()
}
