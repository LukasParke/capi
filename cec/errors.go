package cec

import "errors"

// Sentinel errors returned by the cec package. Callers can use errors.Is to
// classify failures without inspecting strings.
var (
	// ErrClosed is returned by any libcec API call after Close has been called.
	ErrClosed = errors.New("cec: connection closed")

	// ErrNoActiveSource is returned by GetActiveSource when no device on the
	// bus currently claims the active-source role.
	ErrNoActiveSource = errors.New("cec: no active source")

	// ErrInvalidLogicalAddress is returned for addresses outside 0..14.
	ErrInvalidLogicalAddress = errors.New("cec: invalid logical address")

	// ErrInvalidHDMIPort is returned for ports outside 1..15.
	ErrInvalidHDMIPort = errors.New("cec: invalid HDMI port")

	// ErrAdapterNotOpen is returned by API calls before OpenAdapter has been
	// called or after the adapter session has dropped.
	ErrAdapterNotOpen = errors.New("cec: adapter not open")

	// ErrTransmitFailed is returned when libcec rejects a transmit.
	ErrTransmitFailed = errors.New("cec: transmit failed")

	// ErrLibcecCall wraps a generic libcec call failure (the underlying lib
	// returned 0 / NULL).
	ErrLibcecCall = errors.New("cec: libcec call failed")

	// ErrMonitorOnly is returned by Transmit and the SendKey* methods when
	// the connection was opened with Configuration.MonitorOnly = true.
	ErrMonitorOnly = errors.New("cec: connection is monitor-only; cannot transmit")
)
