package main

import (
	"context"
	"log"
	"strings"
	"time"

	"github.com/LukasParke/capi/cec"
)

// runCECSupervisor owns the cec.Connection lifecycle: open libcec, find +
// open the adapter, hand the live connection to the global Adapter, drain
// events, and reconnect on any signal from SignalReconnect or after a backoff
// when something fails.
//
// The previous supervisor used many GOTO labels and an unbounded reconnect
// channel drain loop; this version is a flat for/select with explicit
// state transitions and ctx cancellation so the binary can shut down
// cleanly.
func runCECSupervisor(ctx context.Context, deviceName, requestedAdapter string, monitorFromFlag bool) {
	const minBackoff = 3 * time.Second
	const maxBackoff = 60 * time.Second
	backoff := minBackoff

	for {
		// Drain any pending reconnect signals before attempting to open.
		drain(adapter.reconnectSignal())

		conn, opened, err := openCECSession(ctx, deviceName, requestedAdapter, monitorFromFlag)
		if err != nil {
			if ctx.Err() != nil {
				return
			}
			appLog("supervisor", "session start failed: %v (retry in %s)", err, backoff)
			if !sleepCtx(ctx, backoff) {
				return
			}
			backoff = nextBackoff(backoff, maxBackoff)
			continue
		}
		if !opened {
			// ctx was cancelled mid-open
			if conn != nil {
				_ = conn.Close()
			}
			return
		}

		backoff = minBackoff

		adapter.Set(conn)
		globalBusState.setCECReady(true)
		appLog("supervisor", "adapter session ready (monitor=%v)", monitorFromFlag)
		if eventHub != nil {
			eventHub.Publish(CECEvent{
				Type: "adapter_state",
				Data: map[string]interface{}{"state": "connected"},
			})
		}
		signalStewardFull()
		startMQTTFromConfigIfNeeded()

		// Wait for either a reconnect signal or shutdown.
		select {
		case <-ctx.Done():
			adapter.Set(nil)
			_ = conn.Close()
			globalBusState.setCECReady(false)
			setStewardMonitoringState(false)
			globalBusState.setFrameRingCapacity(0)
			return
		case <-adapter.reconnectSignal():
			appLog("supervisor", "reconnect requested; tearing down adapter session")
		}

		if eventHub != nil {
			eventHub.Publish(CECEvent{
				Type: "adapter_state",
				Data: map[string]interface{}{"state": "disconnected"},
			})
		}

		adapter.Set(nil)
		_ = conn.Close()
		globalBusState.setCECReady(false)
		setStewardMonitoringState(false)
		globalBusState.setFrameRingCapacity(0)

		// Brief delay before retrying to avoid hammering libcec/USB.
		if !sleepCtx(ctx, time.Second) {
			return
		}
	}
}

// openCECSession opens a fresh libcec session and tries to attach to the
// adapter. On success, the runCECEventConsumer is started for the new
// connection so events flow into the rest of the service. Returns
// (conn, true, nil) on success, (conn, false, nil) only if ctx was
// cancelled mid-open. On error, returns nil for the connection.
func openCECSession(ctx context.Context, deviceName, requestedAdapter string, monitorFromFlag bool) (*cec.Connection, bool, error) {
	if ctx.Err() != nil {
		return nil, false, nil
	}

	cecCfg := cec.NewConfiguration(deviceName, cec.DeviceTypeRecordingDevice)
	configMu.RLock()
	persisted := currentConfig.CEC
	configMu.RUnlock()
	cecCfg.MonitorOnly = persisted.MonitorOnly
	cecCfg.ActivateSource = persisted.ActivateSource
	cecCfg.WakeDevices = intsToLogicalAddrs(persisted.WakeOnConnect)
	cecCfg.PowerOffDevices = intsToLogicalAddrs(persisted.PowerOffOnDisconnect)

	conn, err := cec.OpenWith(cecCfg, cec.Options{EventBuffer: 512})
	if err != nil {
		return nil, false, err
	}

	go runCECEventConsumer(conn)

	adapterPath, err := pickAdapter(conn, requestedAdapter)
	if err != nil {
		_ = conn.Close()
		return nil, false, err
	}
	appLog("supervisor", "opening CEC adapter: %s", adapterPath)
	if err := conn.OpenAdapter(adapterPath); err != nil {
		_ = conn.Close()
		return nil, false, err
	}

	configMu.RLock()
	busCfg := currentConfig.Bus
	configMu.RUnlock()

	monitor := monitorFromFlag
	if busCfg.MonitorFromConfig != nil {
		monitor = *busCfg.MonitorFromConfig
	}
	if monitor {
		if err := conn.SwitchMonitoring(true); err != nil {
			appLog("supervisor", "SwitchMonitoring enable failed: %v", err)
		} else {
			appLog("cec", "libCEC monitoring enabled: all bus frames forwarded to OnCommand")
		}
	} else {
		_ = conn.SwitchMonitoring(false)
	}
	setStewardMonitoringState(monitor)
	globalBusState.setFrameRingCapacity(busCfg.frameRingSize())

	// Brief settle so the adapter has logical addresses before the first
	// scan. This used to be a flat 2s sleep on the supervisor goroutine;
	// keep that timing but make it cancel-aware.
	if !sleepCtx(ctx, 2*time.Second) {
		_ = conn.Close()
		return nil, false, nil
	}

	return conn, true, nil
}

func pickAdapter(conn *cec.Connection, requested string) (string, error) {
	if requested != "" {
		return requested, nil
	}
	appLog("supervisor", "searching for CEC adapters")
	adapters, err := conn.FindAdapters()
	if err != nil {
		return "", err
	}
	if len(adapters) == 0 {
		return "", ErrAdapterUnavailable
	}
	first := adapters[0]
	if first.Comm != "" && strings.HasPrefix(first.Comm, "/dev/") {
		return first.Comm, nil
	}
	return first.Path, nil
}

// startMQTTFromConfigIfNeeded reads the persisted MQTT config and (re)starts
// the MQTT bridge if a broker is configured and the existing client is not
// connected. Used by the supervisor whenever a fresh adapter session comes up.
func startMQTTFromConfigIfNeeded() {
	configMu.RLock()
	broker := currentConfig.MQTT.Broker
	user := currentConfig.MQTT.User
	pass := currentConfig.MQTT.Pass
	prefix := currentConfig.MQTT.Prefix
	configMu.RUnlock()

	if broker == "" {
		return
	}
	mqttMu.Lock()
	connected := mqttClient != nil && mqttClient.IsConnected()
	mqttMu.Unlock()
	if connected {
		return
	}
	startMQTT(broker, user, pass, prefix)
}

// sleepCtx blocks for d or until ctx is cancelled. Returns false if ctx was
// cancelled (caller should bail out).
func sleepCtx(ctx context.Context, d time.Duration) bool {
	if d <= 0 {
		return ctx.Err() == nil
	}
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-ctx.Done():
		return false
	case <-t.C:
		return true
	}
}

// nextBackoff doubles a backoff value capped at max.
func nextBackoff(cur, max time.Duration) time.Duration {
	next := cur * 2
	if next > max {
		next = max
	}
	return next
}

// drain consumes all currently-buffered values from a non-blocking signal
// channel without blocking.
func drain(ch <-chan struct{}) {
	for {
		select {
		case <-ch:
		default:
			return
		}
	}
}

// intsToLogicalAddrs converts a JSON-friendly []int to []cec.LogicalAddress,
// dropping anything outside 0..14.
func intsToLogicalAddrs(in []int) []cec.LogicalAddress {
	if len(in) == 0 {
		return nil
	}
	out := make([]cec.LogicalAddress, 0, len(in))
	for _, v := range in {
		if v < 0 || v > 14 {
			continue
		}
		out = append(out, cec.LogicalAddress(v))
	}
	return out
}

var _ = log.Println // retain log import for potential future use
