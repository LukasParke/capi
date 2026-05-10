package main

import (
	"context"
	"errors"
	"sync"
	"sync/atomic"
	"time"

	"github.com/LukasParke/capi/cec"
)

// ErrAdapterUnavailable is returned by Adapter.With when no live cec.Connection
// is currently attached.
var ErrAdapterUnavailable = errors.New("CEC adapter not available")

// Adapter holds the currently live cec.Connection behind an atomic pointer.
// Callers acquire the connection with Conn() and use it directly - the
// underlying cec.Connection serializes libcec calls internally, so multiple
// goroutines may invoke methods on the same Connection concurrently and the
// cec package will queue them safely.
//
// Replacing the previous heavyweight cecMutex with an atomic pointer
// eliminates Go-side lock contention between HTTP handlers, the steward, the
// MQTT bridge, and the libcec callback consumer. The only mutual exclusion
// that remains (around actual libcec entry points) lives inside the cec
// package and is the smallest possible scope.
type Adapter struct {
	current atomic.Pointer[cec.Connection]

	reconnectMu sync.Mutex
	reconnectCh chan struct{}
}

// NewAdapter returns an empty Adapter.
func NewAdapter() *Adapter {
	return &Adapter{
		reconnectCh: make(chan struct{}, 1),
	}
}

// Set attaches a new connection. The previous connection (if any) is NOT
// closed by this call - the supervisor that owns the lifecycle is responsible
// for closing the old one.
func (a *Adapter) Set(conn *cec.Connection) {
	a.current.Store(conn)
}

// Conn returns the live connection or nil if no adapter is attached or the
// attached connection has been closed.
func (a *Adapter) Conn() *cec.Connection {
	c := a.current.Load()
	if c == nil || c.IsClosed() {
		return nil
	}
	return c
}

// Ready reports whether a live connection is attached.
func (a *Adapter) Ready() bool { return a.Conn() != nil }

// With invokes fn with a live connection, returning ErrAdapterUnavailable if
// no adapter is attached.
func (a *Adapter) With(fn func(c *cec.Connection) error) error {
	c := a.Conn()
	if c == nil {
		return ErrAdapterUnavailable
	}
	return fn(c)
}

// SignalReconnect asks the supervisor to tear down the current session and
// reconnect. Multiple concurrent calls coalesce into one signal.
func (a *Adapter) SignalReconnect() {
	select {
	case a.reconnectCh <- struct{}{}:
	default:
	}
}

// reconnectSignal exposes the internal channel to the supervisor.
func (a *Adapter) reconnectSignal() <-chan struct{} { return a.reconnectCh }

// Close clears the adapter pointer and closes the underlying connection.
// Safe to call when nothing is attached.
func (a *Adapter) Close() {
	if c := a.current.Swap(nil); c != nil {
		_ = c.Close()
	}
}

// adapter is the global *Adapter instance. The migration from cecMutex/cecConn
// to this single object is the central simplification of the Phase 2 server
// refactor.
var adapter = NewAdapter()

// withCEC is a convenience that adapts handlers and helpers that don't yet
// take a *cec.Connection directly. Returns ErrAdapterUnavailable when there
// is no attached adapter.
func withCEC(_ context.Context, fn func(c *cec.Connection) error) error {
	return adapter.With(fn)
}

// adapterReady is the canonical "is the adapter usable right now?" check.
func adapterReady() bool { return adapter.Ready() }

// requireAdapter is the inverse of unhealthy "wait" patterns: it returns
// (conn, true) when a live adapter exists, otherwise it writes a 503-style
// failure via the supplied responder and returns (nil, false).
func requireAdapter(fail func()) (*cec.Connection, bool) {
	c := adapter.Conn()
	if c == nil {
		fail()
		return nil, false
	}
	return c, true
}

// pingTV exposes a context-bounded health check for the supervisor.
func adapterPingTV(ctx context.Context) error {
	c := adapter.Conn()
	if c == nil {
		return ErrAdapterUnavailable
	}
	deadline, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	done := make(chan error, 1)
	go func() {
		_, err := c.GetDevicePowerStatus(cec.LogicalAddressTV)
		done <- err
	}()
	select {
	case err := <-done:
		return err
	case <-deadline.Done():
		return deadline.Err()
	}
}
