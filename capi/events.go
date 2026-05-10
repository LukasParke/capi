package main

import (
	"fmt"
	"log"
	"sync"
	"sync/atomic"
	"time"

	"github.com/LukasParke/capi/cec"
)

// CECEvent is the wire format for events emitted by the EventHub. Subscribers
// (SSE, WebSocket, MQTT bridge) decode this directly.
//
// Type values: key_press, command, source_activated, power_change, alert,
// devices_changed, configuration_changed, adapter_state.
type CECEvent struct {
	Type      string      `json:"type"`
	Timestamp time.Time   `json:"timestamp"`
	Data      interface{} `json:"data"`
}

// EventHub is a simple pub/sub hub for CEC events. Subscribers receive events
// on a buffered channel; non-blocking sends mean a slow subscriber drops
// events instead of stalling the publisher (the hub-level dropped/delivered
// counters are exposed via /api/health and /metrics).
type EventHub struct {
	mu         sync.RWMutex
	subs       map[chan CECEvent]struct{}
	bufferSize int

	delivered atomic.Uint64
	dropped   atomic.Uint64
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
	if _, ok := h.subs[ch]; !ok {
		h.mu.Unlock()
		return
	}
	delete(h.subs, ch)
	h.mu.Unlock()
	close(ch)
}

// Publish sends the event to all subscribers without blocking. Slow
// subscribers have the event dropped and the dropped counter incremented.
func (h *EventHub) Publish(ev CECEvent) {
	ev.Timestamp = time.Now()
	h.mu.RLock()
	for ch := range h.subs {
		select {
		case ch <- ev:
			h.delivered.Add(1)
		default:
			h.dropped.Add(1)
		}
	}
	h.mu.RUnlock()
}

// Subscribers returns the current subscriber count.
func (h *EventHub) Subscribers() int {
	h.mu.RLock()
	defer h.mu.RUnlock()
	return len(h.subs)
}

// Stats returns (dropped, delivered) counters.
func (h *EventHub) Stats() (dropped, delivered uint64) {
	return h.dropped.Load(), h.delivered.Load()
}

// LogMessage is a single record in the in-memory log ring buffer surfaced
// at /api/logs and the UI's logs panel.
type LogMessage struct {
	Level     string    `json:"level"`
	Timestamp time.Time `json:"timestamp"`
	Message   string    `json:"message"`
}

// LogHandler is the in-memory ring buffer of recent CEC and application log
// lines. It is fed by the runCECEventConsumer goroutine (RecordCEC) and by
// service code via RecordApp / appLog.
type LogHandler struct {
	LogMessages []LogMessage
	mu          sync.RWMutex
	maxMessages int
}

// NewLogHandler returns a fresh ring buffer of capacity 500.
func NewLogHandler() *LogHandler {
	return &LogHandler{
		LogMessages: make([]LogMessage, 0, 500),
		maxMessages: 500,
	}
}

// RecordApp appends a service-generated log line.
func (l *LogHandler) RecordApp(component, message string) {
	if l == nil {
		return
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	l.LogMessages = append(l.LogMessages, LogMessage{
		Level:     "APP",
		Timestamp: time.Now(),
		Message:   fmt.Sprintf("[%s] %s", component, message),
	})
	if len(l.LogMessages) > l.maxMessages {
		l.LogMessages = l.LogMessages[1:]
	}
}

// RecordCEC appends a libcec log line. Called from the CEC events consumer.
func (l *LogHandler) RecordCEC(level cec.LogLevel, timestamp int64, message string) {
	if l == nil {
		return
	}
	logTime := time.Unix(0, timestamp*int64(time.Millisecond))
	l.mu.Lock()
	l.LogMessages = append(l.LogMessages, LogMessage{
		Level:     level.String(),
		Timestamp: logTime,
		Message:   message,
	})
	if len(l.LogMessages) > l.maxMessages {
		l.LogMessages = l.LogMessages[1:]
	}
	l.mu.Unlock()

	if level != cec.LogLevelTraffic && level != cec.LogLevelDebug {
		log.Printf("[CEC %s] %s", level.String(), message)
	}
}

// GetRecentLogs returns a copy of the current log messages.
func (l *LogHandler) GetRecentLogs() []LogMessage {
	l.mu.RLock()
	defer l.mu.RUnlock()
	out := make([]LogMessage, len(l.LogMessages))
	copy(out, l.LogMessages)
	return out
}

// appLog writes a line to both the systemd journal (via log.Printf) and the
// in-memory ring buffer used by /api/logs and the UI's logs panel.
func appLog(component, format string, args ...interface{}) {
	msg := fmt.Sprintf(format, args...)
	log.Printf("[%s] %s", component, msg)
	if logHandler != nil {
		logHandler.RecordApp(component, msg)
	}
}
