package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"

	mqtt "github.com/eclipse/paho.mqtt.golang"
)

// MQTT bridge: connects to a broker, subscribes to {prefix}/command/# for
// inbound commands, and forwards EventHub events as JSON payloads on
// {prefix}/event/{type} topics. Connect/disconnect/reconnect is handled by
// the underlying paho client; we only own the goroutine that drains the hub
// channel and the lifecycle of the client itself.

var (
	mqttClient mqtt.Client
	mqttMu     sync.Mutex
	mqttCancel context.CancelFunc
)

// stopMQTT disconnects the MQTT client and cancels the event-forwarding goroutine.
// Safe to call when nothing is connected.
func stopMQTT() {
	mqttMu.Lock()
	defer mqttMu.Unlock()
	if mqttCancel != nil {
		mqttCancel()
		mqttCancel = nil
	}
	if mqttClient != nil && mqttClient.IsConnected() {
		mqttClient.Disconnect(1000)
		appLog("mqtt", "disconnected")
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
			appLog("mqtt", "connected broker=%q prefix=%q", broker, prefix)
			cmdTopic := prefix + "/command/#"
			token := c.Subscribe(cmdTopic, 1, func(_ mqtt.Client, msg mqtt.Message) {
				handleMQTTCommand(prefix, msg.Topic(), msg.Payload())
			})
			if token.Wait() && token.Error() != nil {
				appLog("mqtt", "subscribe %q failed: %v", cmdTopic, token.Error())
				log.Printf("[MQTT] Subscribe failed: %v", token.Error())
			} else {
				appLog("mqtt", "subscribed %q", cmdTopic)
				log.Printf("[MQTT] Subscribed to %s", cmdTopic)
			}
		}).
		SetConnectionLostHandler(func(_ mqtt.Client, err error) {
			appLog("mqtt", "connection lost: %v", err)
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

	// Drain EventHub events and forward to {prefix}/event/{type}.
	go runMQTTPublisher(ctx, prefix)
}

// runMQTTPublisher subscribes to the EventHub and republishes each event on
// the corresponding MQTT topic until ctx is cancelled or the hub closes the
// channel.
func runMQTTPublisher(ctx context.Context, prefix string) {
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
}

// handleMQTTCommand dispatches an incoming MQTT message to the appropriate
// CEC operation. Topic format: {prefix}/command/{action}[/{param}]. All
// branches reuse the exec* helpers so JSON, UI, and MQTT commands share the
// same adapter path.
func handleMQTTCommand(prefix, topic string, payload []byte) {
	if !adapterReady() {
		log.Printf("[MQTT] Ignoring command %q: CEC adapter not available", topic)
		return
	}
	cmdPath := strings.TrimPrefix(topic, prefix+"/command/")
	switch cmdPath {
	case "power/on":
		if err := execPowerOn(parseMQTTAddress(payload, 0)); err != nil {
			log.Printf("[MQTT] power/on failed: %v", err)
		}
	case "power/off":
		if err := execPowerOff(parseMQTTAddress(payload, 0)); err != nil {
			log.Printf("[MQTT] power/off failed: %v", err)
		}
	case "volume/up":
		if _, err := execVolumeUp(""); err != nil {
			log.Printf("[MQTT] volume/up failed: %v", err)
		}
	case "volume/down":
		if _, err := execVolumeDown(""); err != nil {
			log.Printf("[MQTT] volume/down failed: %v", err)
		}
	case "volume/mute":
		if _, err := execVolumeMute(""); err != nil {
			log.Printf("[MQTT] volume/mute failed: %v", err)
		}
	case "source":
		if err := execSetActiveSource(parseMQTTAddress(payload, -1)); err != nil {
			log.Printf("[MQTT] source failed: %v", err)
		}
	case "hdmi":
		if err := execHDMIPort(parseMQTTAddress(payload, -1)); err != nil {
			log.Printf("[MQTT] hdmi failed: %v", err)
		}
	case "key":
		var req struct {
			Address int    `json:"address"`
			Key     string `json:"key"`
			Keycode int    `json:"keycode"`
		}
		if err := json.Unmarshal(payload, &req); err != nil {
			log.Printf("[MQTT] key: invalid payload: %v", err)
			return
		}
		if err := execSendKey(req.Address, req.Key, req.Keycode); err != nil {
			log.Printf("[MQTT] key failed: %v", err)
		}
	default:
		log.Printf("[MQTT] Unknown command topic: %s", topic)
	}
}

// parseMQTTAddress parses a simple integer from the payload.
// Returns defaultVal if the payload is empty or malformed.
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

// HTTP handlers for the MQTT settings panel.

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
	if err := applyMQTTSettings(MQTTConfig{
		Broker: req.Broker,
		User:   req.User,
		Pass:   req.Pass,
		Prefix: req.Prefix,
	}); err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, "MQTT settings saved", nil)
}
