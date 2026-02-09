package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"

	"capi/cec"

	"github.com/gorilla/mux"
)

var (
	cecConn    *cec.Connection
	cecMutex   sync.Mutex
	logHandler *LogHandler
)

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
}

func (l *LogHandler) OnCommand(command *cec.Command) {
	log.Printf("Command received: %s -> %s, opcode: 0x%02X",
		command.Initiator.String(), command.Destination.String(), command.Opcode)
}

func (l *LogHandler) OnConfigurationChanged(config *cec.Configuration) {
	log.Printf("Configuration changed: %s", config.DeviceName)
}

func (l *LogHandler) OnAlert(alert cec.Alert, param cec.Parameter) {
	log.Printf("Alert: %d", alert)
}

func (l *LogHandler) OnMenuStateChanged(state cec.MenuState) bool {
	log.Printf("Menu state changed: %d", state)
	return true
}

func (l *LogHandler) OnSourceActivated(address cec.LogicalAddress, activated bool) {
	log.Printf("Source activated: %s, activated: %v", address.String(), activated)
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

// Device endpoints

func getDevicesHandler(w http.ResponseWriter, r *http.Request) {
	// Optionally force a rescan when requested by the client.
	rescanParam := r.URL.Query().Get("rescan")

	cecMutex.Lock()
	var (
		devices []*cec.Device
		err     error
	)
	if rescanParam == "1" || strings.EqualFold(rescanParam, "true") {
		devices, err = cecConn.GetAllDevices()
	} else {
		devices, err = cecConn.GetAllDevicesNoRescan()
	}
	cecMutex.Unlock()

	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	// Convert to JSON-friendly format
	result := make([]map[string]interface{}, len(devices))
	for i, dev := range devices {
		result[i] = map[string]interface{}{
			"logical_address":  int(dev.LogicalAddress),
			"address_name":     dev.LogicalAddress.String(),
			"physical_address": cec.PhysicalAddressToString(dev.PhysicalAddress),
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

	respondSuccess(w, "Devices retrieved", result)
}

func getDeviceHandler(w http.ResponseWriter, r *http.Request) {
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
	cecMutex.Lock()
	defer cecMutex.Unlock()

	err := cecConn.VolumeUp(true)
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, "Volume up command sent", nil)
}

func volumeDownHandler(w http.ResponseWriter, r *http.Request) {
	cecMutex.Lock()
	defer cecMutex.Unlock()

	err := cecConn.VolumeDown(true)
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, "Volume down command sent", nil)
}

func muteHandler(w http.ResponseWriter, r *http.Request) {
	cecMutex.Lock()
	defer cecMutex.Unlock()

	err := cecConn.AudioToggleMute()
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}

	respondSuccess(w, "Mute toggle command sent", nil)
}

// Source control endpoints

func getActiveSourceHandler(w http.ResponseWriter, r *http.Request) {
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
	vars := mux.Vars(r)
	portStr := vars["port"]

	port, err := strconv.Atoi(portStr)
	if err != nil || port < 1 || port > 4 {
		respondError(w, http.StatusBadRequest, "Invalid HDMI port (must be 1-4)")
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

// Health check

func healthHandler(w http.ResponseWriter, r *http.Request) {
	cecMutex.Lock()
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
		"version": "1.0.0",
		"libcec":  libInfo,
	})
}

func main() {
	bindAddr := flag.String("bind", ":8080", "Bind address (e.g., :8080 for all interfaces, localhost:8080 for local only)")
	deviceName := flag.String("name", "CEC HTTP Bridge", "Device name")
	adapterPath := flag.String("adapter", "", "CEC adapter path (auto-detect if empty)")
	flag.Parse()

	// Initialize CEC
	log.Println("Initializing CEC connection...")
	var err error
	cecConn, err = cec.Open(*deviceName, cec.DeviceTypeRecordingDevice)
	if err != nil {
		log.Fatalf("Failed to initialize CEC: %v", err)
	}
	defer cecConn.Close()

	// Set up logging
	logHandler = NewLogHandler()
	cecConn.SetCallbackHandler(logHandler)

	// Find and open adapter
	if *adapterPath == "" {
		log.Println("Searching for CEC adapters...")
		adapters, err := cecConn.FindAdapters()
		if err != nil || len(adapters) == 0 {
			log.Fatalf("No CEC adapters found")
		}
		*adapterPath = adapters[0].Path
		log.Printf("Found adapter: %s (%s)", adapters[0].Path, adapters[0].Comm)
	}

	log.Printf("Opening CEC adapter: %s", *adapterPath)
	if err := cecConn.OpenAdapter(*adapterPath); err != nil {
		log.Fatalf("Failed to open CEC adapter: %v", err)
	}

	log.Println("CEC connection established")
	log.Println(cecConn.GetLibInfo())

	// Wait for CEC to initialize
	time.Sleep(2 * time.Second)

	// Set up HTTP router
	r := mux.NewRouter()

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
	r.HandleFunc("/api/volume/down", volumeDownHandler).Methods("POST")
	r.HandleFunc("/api/volume/mute", muteHandler).Methods("POST")

	// Source control
	r.HandleFunc("/api/source/active", getActiveSourceHandler).Methods("GET")
	r.HandleFunc("/api/source/{address}", setActiveSourceHandler).Methods("POST")
	r.HandleFunc("/api/hdmi/{port}", setHDMIPortHandler).Methods("POST")

	// Navigation
	r.HandleFunc("/api/key", sendKeyHandler).Methods("POST")

	// Raw command
	r.HandleFunc("/api/command", rawCommandHandler).Methods("POST")

	// Logs
	r.HandleFunc("/api/logs", getLogsHandler).Methods("GET")

	// Health
	r.HandleFunc("/api/health", healthHandler).Methods("GET")

	// Start server
	log.Printf("Starting HTTP server on %s", *bindAddr)
	log.Printf("API documentation: http://%s/api/health", *bindAddr)

	if err := http.ListenAndServe(*bindAddr, r); err != nil {
		log.Fatalf("Server failed: %v", err)
	}
}
