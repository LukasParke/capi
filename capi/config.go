package main

import (
	"encoding/json"
	"os"
	"sync"
)

// MQTTConfig holds MQTT broker connection settings persisted in config.json.
type MQTTConfig struct {
	Broker string `json:"broker"`
	User   string `json:"user"`
	Pass   string `json:"pass"`
	Prefix string `json:"prefix"`
}

// CECConfig persists the bus-disruption knobs that the cec package exposes.
// Defaults are the safe ones: passive observer, no auto-active-source, no
// wake/standby on the adapter session.
type CECConfig struct {
	// MonitorOnly: when true, libcec doesn't allocate a logical address and
	// the connection becomes a pure read-only listener. Toggleable at
	// runtime via POST /api/dev/mode (triggers a reconnect).
	MonitorOnly bool `json:"monitor_only"`

	// ActivateSource: when true, libcec announces this connection as the
	// active source on libcec_open. Defaults to false to avoid hijacking
	// the user's display input on every reconnect.
	ActivateSource bool `json:"activate_source"`

	// WakeOnConnect: logical addresses libcec wakes on connect (sends
	// ImageViewOn / ActiveSource). Empty by default.
	WakeOnConnect []int `json:"wake_on_connect"`

	// PowerOffOnDisconnect: logical addresses libcec puts in standby on
	// disconnect (broadcast Standby). Empty by default.
	PowerOffOnDisconnect []int `json:"power_off_on_disconnect"`

	// StrategyOverrides selects a single named strategy as the per-vendor
	// default for an action. Vendors are lowercase 0x-prefixed hex
	// (e.g. "0x000048"); actions are the canonical lowercase names from
	// Action.String().
	//
	// {"0x000048": {"volume_up": "uc_volume_up_tv"}}
	StrategyOverrides map[string]map[string]string `json:"strategy_overrides,omitempty"`
}

// Config is the on-disk configuration file format. The "bus" section feeds
// busstate.go's BusConfig (reconcile interval, frame ring size, vendor
// profiles); the "cec" section feeds the cec.Configuration that the
// supervisor passes to cec.OpenWith on every reconnect.
type Config struct {
	MQTT MQTTConfig `json:"mqtt"`
	Bus  BusConfig  `json:"bus"`
	CEC  CECConfig  `json:"cec"`
}

// Globals: the live config is loaded at boot and replaced under configMu when
// the user updates settings via the UI/API. Files on disk live at
// configFilePath (next to the binary by default).
var (
	currentConfig  Config
	configMu       sync.RWMutex
	configFilePath string
)

// loadConfig reads and parses the config file. Returns zero Config if the
// file does not exist or fails to parse, so the binary can always start
// even with no on-disk config.
func loadConfig(path string) Config {
	var cfg Config
	data, err := os.ReadFile(path)
	if err != nil {
		return cfg
	}
	_ = json.Unmarshal(data, &cfg)
	return cfg
}

// saveConfig atomically writes the config file via temp file + rename.
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

// applyPersistedStrategyOverrides walks currentConfig.CEC.StrategyOverrides
// and installs each named override into the defaultRegistry. Logs (but
// doesn't fail on) entries that reference an unknown action or strategy.
// Called from main() after the config is loaded.
func applyPersistedStrategyOverrides() {
	configMu.RLock()
	overrides := currentConfig.CEC.StrategyOverrides
	configMu.RUnlock()
	if len(overrides) == 0 {
		return
	}
	for vendor, byAction := range overrides {
		for actionName, stratName := range byAction {
			action, ok := ParseAction(actionName)
			if !ok {
				appLog("config", "skipping strategy override: unknown action %q", actionName)
				continue
			}
			var picked *Strategy
			for _, s := range defaultRegistry.StrategiesFor("", action) {
				if s.Name == stratName {
					s := s
					picked = &s
					break
				}
			}
			if picked == nil {
				appLog("config", "skipping strategy override %s/%s: strategy %q not found", vendor, actionName, stratName)
				continue
			}
			defaultRegistry.SetVendorOverride(vendor, action, []Strategy{*picked})
			appLog("config", "applied strategy override vendor=%s action=%s strategy=%s", vendor, actionName, stratName)
		}
	}
}
