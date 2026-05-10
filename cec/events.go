package cec

import (
	"sync/atomic"
	"time"
)

// EventKind enumerates the kinds of asynchronous events emitted by libcec.
type EventKind int

const (
	EventInvalid EventKind = iota
	EventLog
	EventKeyPress
	EventCommand
	EventConfigChanged
	EventAlert
	EventSourceActivated
)

func (k EventKind) String() string {
	switch k {
	case EventLog:
		return "log"
	case EventKeyPress:
		return "key_press"
	case EventCommand:
		return "command"
	case EventConfigChanged:
		return "config_changed"
	case EventAlert:
		return "alert"
	case EventSourceActivated:
		return "source_activated"
	default:
		return "invalid"
	}
}

// LogPayload carries a libcec log message.
type LogPayload struct {
	Level   LogLevel
	Time    int64
	Message string
}

// KeyPayload carries a remote key press.
type KeyPayload struct {
	Key      Keycode
	Duration uint32
}

// AlertPayload carries an adapter alert.
type AlertPayload struct {
	Alert Alert
	Param Parameter
}

// SourcePayload carries a source-activation change.
type SourcePayload struct {
	Address   LogicalAddress
	Activated bool
}

// Event is the unified type emitted on the channel returned by Connection.Events().
// Exactly one payload pointer is non-nil for any given Event, matched to Kind.
type Event struct {
	Kind      EventKind
	Timestamp time.Time

	Log     *LogPayload
	Key     *KeyPayload
	Command *Command
	Alert   *AlertPayload
	Source  *SourcePayload
	Config  *Configuration
}

// EventStats tracks event-channel health (atomic counters).
type EventStats struct {
	delivered atomic.Uint64
	dropped   atomic.Uint64
}

// Delivered returns the total number of events successfully posted to the channel.
func (s *EventStats) Delivered() uint64 { return s.delivered.Load() }

// Dropped returns the total number of events that were dropped because the
// channel was full (i.e. the consumer was slower than the libcec callback rate).
func (s *EventStats) Dropped() uint64 { return s.dropped.Load() }
