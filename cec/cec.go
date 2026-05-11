// Package cec provides idiomatic Go bindings for libcec.
//
// A Connection owns one libcec session. Construct it with Open, attach to a
// physical adapter with OpenAdapter, consume asynchronous events from the
// channel returned by Events, and tear it down with Close.
//
// All exported methods on *Connection that drive libcec are serialized
// internally via a single mutex; you may call them from multiple goroutines.
// Callbacks (and the events emitted on the Events channel) fire on libcec's
// own threads and never wait on the API mutex, so they will not deadlock with
// in-flight API calls.
package cec

/*
#cgo pkg-config: libcec
#cgo LDFLAGS: -Wl,--no-as-needed -lstdc++ -Wl,--as-needed
#include <libcec/cecc.h>
#include <stdlib.h>
#include <stdint.h>

extern void cec_install_callbacks(libcec_configuration* cfg, uintptr_t handle);
extern void cec_set_passive_defaults(libcec_configuration* cfg);
extern void cec_set_activate_source(libcec_configuration* cfg, int v);
extern void cec_set_monitor_only(libcec_configuration* cfg, int v);
extern void cec_apply_address_list(cec_logical_addresses* dest, const uint8_t* addrs, int n);
*/
import "C"

import (
	"errors"
	"fmt"
	"runtime/cgo"
	"sync"
	"sync/atomic"
	"unsafe"
)

// Configuration mirrors a subset of libcec_configuration that is meaningful
// to typical bridge use. Use NewConfiguration to obtain one with sensible
// (passive, non-disruptive) defaults; pass to OpenWith.
type Configuration struct {
	DeviceName        string
	DeviceType        DeviceType
	PhysicalAddress   uint16
	BaseDevice        LogicalAddress
	HDMIPort          uint8
	ClientVersion     uint32
	ServerVersion     uint32
	TryLogicalAddress LogicalAddress

	// ActivateSource: when true, libcec announces this connection as the
	// active source on libcec_open, which causes a TV/projector to switch
	// its input to us. Defaults to false; opt in only when you actually want
	// to claim the display.
	ActivateSource bool

	// WakeDevices: logical addresses libcec wakes on connect (sends
	// ImageViewOn / ActiveSource). Empty by default; libcec's default of
	// {TV} is suppressed.
	WakeDevices []LogicalAddress

	// PowerOffDevices: logical addresses libcec puts in standby on
	// disconnect (broadcast Standby). Empty by default; libcec's default of
	// {BROADCAST} is suppressed so the entire bus does not standby when the
	// adapter session ends.
	PowerOffDevices []LogicalAddress

	// MonitorOnly: when true, libcec does not allocate a logical address
	// and we become a pure read-only listener. Transmit calls return
	// ErrMonitorOnly while in this mode.
	MonitorOnly bool
}

// Options configure a new Connection. Zero-value Options are valid.
type Options struct {
	// EventBuffer is the capacity of the buffered Events channel. Events that
	// arrive while the channel is full are dropped and counted in EventStats.
	// Defaults to 256 if 0.
	EventBuffer int
}

// Connection represents one libcec session.
type Connection struct {
	handle      C.libcec_connection_t
	cgoHandle   cgo.Handle
	config      *Configuration
	apiMu       sync.Mutex
	closed      atomic.Bool
	initialized bool

	events chan Event
	stats  EventStats
	menuFn atomic.Pointer[func(MenuState) bool]
}

// NewConfiguration returns a default configuration suitable for a passive
// CEC bridge: auto-detect physical address, libcec client version we were
// built against, and explicitly non-disruptive (no active-source claim, no
// wake on connect, no standby broadcast on disconnect).
func NewConfiguration(deviceName string, deviceType DeviceType) *Configuration {
	return &Configuration{
		DeviceName:        deviceName,
		DeviceType:        deviceType,
		PhysicalAddress:   0xFFFF, // auto-detect
		ClientVersion:     C.LIBCEC_VERSION_CURRENT,
		TryLogicalAddress: LogicalAddressUnknown,
		// Bus-disruption knobs default off. Override on the returned struct
		// before calling OpenWith if you actually want libcec to take over
		// the display / wake the TV / standby the bus.
		ActivateSource:  false,
		WakeDevices:     nil,
		PowerOffDevices: nil,
		MonitorOnly:     false,
	}
}

// Open creates a new CEC connection with default options.
func Open(deviceName string, deviceType DeviceType) (*Connection, error) {
	return OpenWith(NewConfiguration(deviceName, deviceType), Options{})
}

// OpenWith creates a new CEC connection with the given configuration and options.
func OpenWith(cfg *Configuration, opts Options) (*Connection, error) {
	if cfg == nil {
		return nil, errors.New("cec: nil Configuration")
	}
	bufSize := opts.EventBuffer
	if bufSize <= 0 {
		bufSize = 256
	}
	c := &Connection{
		config: cfg,
		events: make(chan Event, bufSize),
	}
	c.cgoHandle = cgo.NewHandle(c)

	cConfig := C.libcec_configuration{}
	cName := buildLibCECConfig(&cConfig, cfg)
	defer C.free(unsafe.Pointer(cName))

	C.cec_install_callbacks(&cConfig, C.uintptr_t(c.cgoHandle))

	c.handle = C.libcec_initialise(&cConfig)
	if c.handle == nil {
		c.cgoHandle.Delete()
		return nil, fmt.Errorf("%w: libcec_initialise", ErrLibcecCall)
	}

	c.initialized = true
	return c, nil
}

// buildLibCECConfig populates a freshly-zeroed libcec_configuration from the
// Go-side Configuration, applying our passive defaults first and then
// overriding only the knobs the caller explicitly requested. The returned
// *C.char is the strDeviceName malloc the caller must free.
func buildLibCECConfig(cConfig *C.libcec_configuration, cfg *Configuration) *C.char {
	C.libcec_clear_configuration(cConfig)
	// Override libcec's bus-disrupting defaults
	// (bActivateSource=1, wakeDevices={TV}, powerOffDevices={BROADCAST}).
	C.cec_set_passive_defaults(cConfig)

	cName := C.CString(cfg.DeviceName)
	C.strncpy(&cConfig.strDeviceName[0], cName, C.LIBCEC_OSD_NAME_SIZE-1)

	cConfig.deviceTypes.types[0] = C.cec_device_type(cfg.DeviceType)
	cConfig.iPhysicalAddress = C.uint16_t(cfg.PhysicalAddress)
	cConfig.baseDevice = C.cec_logical_address(cfg.BaseDevice)
	cConfig.iHDMIPort = C.uint8_t(cfg.HDMIPort)
	cConfig.clientVersion = C.uint32_t(cfg.ClientVersion)

	if cfg.ActivateSource {
		C.cec_set_activate_source(cConfig, 1)
	}
	if cfg.MonitorOnly {
		C.cec_set_monitor_only(cConfig, 1)
	}
	applyAddressList(&cConfig.wakeDevices, cfg.WakeDevices)
	applyAddressList(&cConfig.powerOffDevices, cfg.PowerOffDevices)

	return cName
}

// applyAddressList writes a Go LogicalAddress slice into a C
// cec_logical_addresses field. An empty slice clears it (no addresses).
func applyAddressList(dest *C.cec_logical_addresses, addrs []LogicalAddress) {
	if dest == nil {
		return
	}
	if len(addrs) == 0 {
		C.cec_apply_address_list(dest, nil, 0)
		return
	}
	buf := make([]C.uint8_t, 0, len(addrs))
	for _, la := range addrs {
		if !la.IsValid() {
			continue
		}
		buf = append(buf, C.uint8_t(la))
	}
	if len(buf) == 0 {
		C.cec_apply_address_list(dest, nil, 0)
		return
	}
	C.cec_apply_address_list(dest, &buf[0], C.int(len(buf)))
}

// IsMonitorOnly reports whether this connection was opened with
// MonitorOnly=true. In that mode libcec did not allocate a logical address
// and Transmit / SendKeypress / SendKeyRelease will refuse with
// ErrMonitorOnly.
func (c *Connection) IsMonitorOnly() bool {
	if c == nil || c.config == nil {
		return false
	}
	return c.config.MonitorOnly
}

// Events returns a channel that delivers asynchronous CEC events. The channel
// is created at Open and closed at Close. A single consumer goroutine should
// drain it; events that arrive while the channel is full are dropped and
// counted in EventStats.
func (c *Connection) Events() <-chan Event { return c.events }

// EventStats returns the live event-channel counters (delivered, dropped).
func (c *Connection) EventStats() *EventStats { return &c.stats }

// SetMenuStateHandler installs an optional callback that decides how libcec
// reports CEC menu state changes. Pass nil to remove the handler. The callback
// runs on a libcec thread and must not block.
func (c *Connection) SetMenuStateHandler(fn func(MenuState) bool) {
	if fn == nil {
		c.menuFn.Store(nil)
		return
	}
	f := fn
	c.menuFn.Store(&f)
}

// dispatch posts an event to the Events channel without blocking.
// If the channel is full, the event is dropped and the stats counter incremented.
func (c *Connection) dispatch(ev Event) {
	select {
	case c.events <- ev:
		c.stats.delivered.Add(1)
	default:
		c.stats.dropped.Add(1)
	}
}

// Close releases all resources associated with the connection. It is safe to
// call multiple times; subsequent calls return nil. After Close, every API
// method returns ErrClosed.
func (c *Connection) Close() error {
	if !c.closed.CompareAndSwap(false, true) {
		return nil
	}
	c.apiMu.Lock()
	defer c.apiMu.Unlock()

	if c.initialized {
		C.libcec_close(c.handle)
		C.libcec_destroy(c.handle)
		c.initialized = false
	}
	c.cgoHandle.Delete()
	close(c.events)
	return nil
}

// IsClosed reports whether Close has been called.
func (c *Connection) IsClosed() bool { return c.closed.Load() }

// guard is the standard prologue for libcec API calls: refuse if closed,
// otherwise lock apiMu. Returns true if the caller should defer apiMu.Unlock.
func (c *Connection) guard() error {
	if c.closed.Load() {
		return ErrClosed
	}
	c.apiMu.Lock()
	if c.closed.Load() {
		c.apiMu.Unlock()
		return ErrClosed
	}
	return nil
}
