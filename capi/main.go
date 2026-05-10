// Command capi is the HDMI-CEC HTTP/MQTT bridge.
//
// The binary is organized as a single package main split across focused
// files (see Phase 2 of the refactor):
//
//   main.go             - entry point; flag parsing, lifecycle wiring
//   server.go           - HTTP router setup, ListenAndServe, shutdown
//   adapter.go          - Adapter type wrapping the live cec.Connection
//   supervisor.go       - background goroutine that owns adapter lifecycle
//   cec_events.go       - drain cec.Connection.Events() into bus state + hub
//   cec_exec.go         - shared CEC command helpers
//   cec_topology.go     - HDMI topology builder (was in cec/, now app-side)
//   busstate.go         - cached steward snapshot + observed-from-traffic merge
//   steward.go          - bounded queue of light/full/deep bus rebuild jobs
//   topology.go         - opcode-to-steward-tier classifier + worker
//   devices_fetch.go    - non-blocking GET /api/devices semantics
//   handlers_devices.go - JSON handlers for devices/bus/topology
//   handlers_control.go - JSON handlers for power/volume/source/key/raw/etc
//   sse.go              - Server-Sent Events stream
//   ws_events.go        - HTMX OOB-fragment WebSocket stream (coalesced)
//   ui.go               - HTML fragments + UI action endpoints (htmx)
//   middleware.go       - recovery / request id / access log + /metrics
//   mqtt.go             - MQTT broker bridge
//   mqtt_apply.go       - MQTT settings persistence
//   config.go           - on-disk config.json
//   update.go           - GitHub release self-update
//   events.go           - EventHub + LogHandler + appLog
//   httpx.go            - JSON Response envelope helpers
//   embed.go            - go:embed directives for templates and static
package main

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"
)

// version is set at build time via -ldflags "-X main.version=...".
var version = "dev"

// logHandler and eventHub are global because they are touched by handlers,
// MQTT, the supervisor, the steward, and the CEC event consumer. They are
// initialized in main() before any other goroutine starts.
var (
	logHandler *LogHandler
	eventHub   *EventHub
)

// signalCECReconnect is a top-level shim for code (cec_events.go,
// supervisor.go) that wants to ask the supervisor for a session restart
// without depending on the Adapter type directly.
func signalCECReconnect() { adapter.SignalReconnect() }

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
	cecBusMonitor := flag.Bool("cec-monitor", false, "Enable libCEC monitoring: every CEC frame triggers OnCommand (very verbose; use with journald / make logs-pi)")
	flag.Parse()

	if *showVersion {
		fmt.Println(version)
		os.Exit(0)
	}
	if *doUpdate {
		doSelfUpdate()
		return
	}

	// Config: file next to the binary, CLI flags override file values.
	exe, _ := os.Executable()
	configFilePath = filepath.Join(filepath.Dir(exe), "config.json")
	currentConfig = loadConfig(configFilePath)
	overlayCLIConfig(*mqttBroker, *mqttUser, *mqttPass, *mqttPrefix)

	// Independent of CEC: log buffer + event hub run from boot.
	eventHub = NewEventHub(64)
	logHandler = NewLogHandler()

	// Background workers that don't need the adapter to be ready.
	startBusStewardIfNeeded()
	go runBusTopologyWorkerLoop()

	runServer(*bindAddr, *deviceName, *adapterPath, *cecBusMonitor)
}

// overlayCLIConfig merges CLI flags into the persisted config. CLI flags
// take priority over the file. Empty CLI values keep whatever is on disk.
// The "mqtt-prefix" flag is special: only override if the user explicitly
// set it (so the empty default doesn't wipe a configured prefix).
func overlayCLIConfig(broker, user, pass, prefix string) {
	if broker != "" {
		currentConfig.MQTT.Broker = broker
	}
	if user != "" {
		currentConfig.MQTT.User = user
	}
	if pass != "" {
		currentConfig.MQTT.Pass = pass
	}
	flag.Visit(func(f *flag.Flag) {
		if f.Name == "mqtt-prefix" {
			currentConfig.MQTT.Prefix = prefix
		}
	})
	if currentConfig.MQTT.Prefix == "" {
		currentConfig.MQTT.Prefix = "capi"
	}
}
