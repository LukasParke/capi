package main

import (
	"fmt"
	"strings"
)

// applyMQTTSettings updates persisted MQTT config, restarts the MQTT client when a
// broker is set, and disconnects when the broker is cleared. Password handling
// matches the JSON API: empty clears; "***" preserves the existing secret.
func applyMQTTSettings(in MQTTConfig) error {
	broker := strings.TrimSpace(in.Broker)
	user := strings.TrimSpace(in.User)
	prefix := strings.TrimSpace(in.Prefix)
	if prefix == "" {
		prefix = "capi"
	}

	configMu.Lock()
	cfg := currentConfig
	cfg.MQTT.Broker = broker
	cfg.MQTT.User = user
	switch in.Pass {
	case "***":
		// masked placeholder from GET /api/settings/mqtt — leave unchanged
	case "":
		cfg.MQTT.Pass = ""
	default:
		cfg.MQTT.Pass = in.Pass
	}
	cfg.MQTT.Prefix = prefix
	currentConfig = cfg
	path := configFilePath
	mqttUser := cfg.MQTT.User
	mqttPass := cfg.MQTT.Pass
	configMu.Unlock()

	if err := saveConfig(path, cfg); err != nil {
		return fmt.Errorf("save config: %w", err)
	}

	if broker != "" {
		startMQTT(broker, mqttUser, mqttPass, prefix)
	} else {
		stopMQTT()
	}
	return nil
}
