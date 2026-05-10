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

// Config is the on-disk configuration file format. The "bus" section feeds
// busstate.go's BusConfig, including periodic reconcile interval, frame ring
// size, and per-vendor probe profiles.
type Config struct {
	MQTT MQTTConfig `json:"mqtt"`
	Bus  BusConfig  `json:"bus"`
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
