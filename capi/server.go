package main

import (
	"context"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/gorilla/mux"
)

// runServer wires up routes, starts the CEC supervisor, the HTTP server,
// and waits for SIGINT/SIGTERM before draining everything cleanly.
func runServer(bindAddr, deviceName, adapterPath string, cecBusMonitor bool) {
	supervisorCtx, stopSupervisor := context.WithCancel(context.Background())
	go runCECSupervisor(supervisorCtx, deviceName, adapterPath, cecBusMonitor)
	defer stopSupervisor()

	server := &http.Server{
		Addr:              bindAddr,
		Handler:           buildRouter(),
		ReadHeaderTimeout: 15 * time.Second,
	}

	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM)

	go func() {
		log.Printf("Starting HTTP server on %s", bindAddr)
		log.Printf("API documentation: http://%s/api/health", bindAddr)
		appLog("http", "listening on %s version=%s", bindAddr, version)
		if err := server.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			log.Fatalf("Server failed: %v", err)
		}
	}()

	<-sigChan
	log.Println("Shutting down...")
	stopMQTT()
	stopSupervisor()

	shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if err := server.Shutdown(shutdownCtx); err != nil {
		log.Printf("HTTP server shutdown: %v", err)
	}
	adapter.Close()
}

// buildRouter constructs the gorilla/mux router with the standard middleware
// stack (request id, panic recovery, structured access log) and registers
// every API/UI endpoint.
func buildRouter() http.Handler {
	r := mux.NewRouter()
	r.Use(
		mux.MiddlewareFunc(requestIDMiddleware),
		mux.MiddlewareFunc(recoverMiddleware),
		mux.MiddlewareFunc(loggingMiddleware),
	)

	registerUIHandlers(r)
	registerAPIRoutes(r)
	return r
}

// registerAPIRoutes attaches every /api/* (and /metrics) handler.
func registerAPIRoutes(r *mux.Router) {
	// Devices and bus state
	r.HandleFunc("/api/devices", getDevicesHandler).Methods("GET")
	r.HandleFunc("/api/devices/{address}", getDeviceHandler).Methods("GET")
	r.HandleFunc("/api/bus/state", getBusStateHandler).Methods("GET")
	r.HandleFunc("/api/bus/scan", postBusScanHandler).Methods("POST")
	r.HandleFunc("/api/bus/frames", getBusFramesHandler).Methods("GET")
	r.HandleFunc("/api/topology", getTopologyHandler).Methods("GET")

	// Power
	r.HandleFunc("/api/power/on", powerOnHandler).Methods("POST")
	r.HandleFunc("/api/power/on/{address}", powerOnHandler).Methods("POST")
	r.HandleFunc("/api/power/off", powerOffHandler).Methods("POST")
	r.HandleFunc("/api/power/off/{address}", powerOffHandler).Methods("POST")
	r.HandleFunc("/api/power/status", getPowerStatusHandler).Methods("GET")
	r.HandleFunc("/api/power/status/{address}", getPowerStatusHandler).Methods("GET")

	// Volume
	r.HandleFunc("/api/volume/up", volumeUpHandler).Methods("POST")
	r.HandleFunc("/api/volume/up/{address}", volumeUpHandler).Methods("POST")
	r.HandleFunc("/api/volume/down", volumeDownHandler).Methods("POST")
	r.HandleFunc("/api/volume/down/{address}", volumeDownHandler).Methods("POST")
	r.HandleFunc("/api/volume/mute", muteHandler).Methods("POST")
	r.HandleFunc("/api/volume/mute/{address}", muteHandler).Methods("POST")

	// Source
	r.HandleFunc("/api/source/active", getActiveSourceHandler).Methods("GET")
	r.HandleFunc("/api/source/{address}", setActiveSourceHandler).Methods("POST")
	r.HandleFunc("/api/hdmi/{port}", setHDMIPortHandler).Methods("POST")

	// Audio
	r.HandleFunc("/api/audio/status", getAudioStatusHandler).Methods("GET")

	// Navigation + raw
	r.HandleFunc("/api/key", sendKeyHandler).Methods("POST")
	r.HandleFunc("/api/command", rawCommandHandler).Methods("POST")

	// Logs + events
	r.HandleFunc("/api/logs", getLogsHandler).Methods("GET")
	r.HandleFunc("/api/events", eventsSSEHandler).Methods("GET")
	r.HandleFunc("/api/events/ws", eventsWebSocketHandler)

	// Health, metrics, update
	r.HandleFunc("/api/health", healthHandler).Methods("GET")
	r.HandleFunc("/metrics", metricsHandler).Methods("GET")
	r.HandleFunc("/api/update", updateHandler).Methods("POST")

	// MQTT settings
	r.HandleFunc("/api/settings/mqtt", getMQTTSettingsHandler).Methods("GET")
	r.HandleFunc("/api/settings/mqtt", postMQTTSettingsHandler).Methods("POST")

	// Dev / unstable surface (Phase 1+ work). Documented as unstable;
	// intended for the /dev page and ad-hoc CEC investigation.
	r.HandleFunc("/api/dev/mode", getDevModeHandler).Methods("GET")
	r.HandleFunc("/api/dev/mode", postDevModeHandler).Methods("POST")
	r.HandleFunc("/api/dev/probe", postDevProbeHandler).Methods("POST")
	r.HandleFunc("/api/dev/send_key", postDevSendKeyHandler).Methods("POST")
	r.HandleFunc("/api/dev/send_opcode", postDevSendOpcodeHandler).Methods("POST")
	r.HandleFunc("/api/dev/run_strategies", postDevRunStrategiesHandler).Methods("POST")
	r.HandleFunc("/api/dev/save_strategy", postDevSaveStrategyHandler).Methods("POST")
	r.HandleFunc("/api/dev/actions", getDevActionsHandler).Methods("GET")
	r.HandleFunc("/api/dev/keys", getDevKeysHandler).Methods("GET")
	r.HandleFunc("/api/dev/opcodes", getDevOpcodesHandler).Methods("GET")
}
