package cec

/*
#include <libcec/cecc.h>
#include <stdlib.h>
#include <stdint.h>

extern void cec_install_callbacks(libcec_configuration* cfg, uintptr_t handle);
*/
import "C"

import (
	"context"
	"fmt"
	"time"
	"unsafe"
)

// FindAdapters lists available CEC adapters reachable from this libcec session.
func (c *Connection) FindAdapters() ([]Adapter, error) {
	if err := c.guard(); err != nil {
		return nil, err
	}
	defer c.apiMu.Unlock()

	var adapters [10]C.cec_adapter
	count := C.libcec_find_adapters(c.handle, &adapters[0], 10, nil)
	if count < 0 {
		return nil, fmt.Errorf("%w: libcec_find_adapters", ErrLibcecCall)
	}
	out := make([]Adapter, count)
	for i := 0; i < int(count); i++ {
		out[i] = Adapter{
			Path: C.GoString(&adapters[i].path[0]),
			Comm: C.GoString(&adapters[i].comm[0]),
		}
	}
	return out, nil
}

// OpenAdapter opens a connection to the given adapter path. If a previous
// adapter session is still open, it is closed first.
func (c *Connection) OpenAdapter(adapterPath string) error {
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()

	cPath := C.CString(adapterPath)
	defer C.free(unsafe.Pointer(cPath))

	// libcec_open is idempotent in name but not always in behavior across
	// versions; explicitly close any prior session first.
	C.libcec_close(c.handle)

	if C.libcec_open(c.handle, cPath, 5000) == 0 {
		return fmt.Errorf("%w: libcec_open(%s)", ErrAdapterNotOpen, adapterPath)
	}
	return nil
}

// PowerOn powers on a device. address must be 0..14.
func (c *Connection) PowerOn(address LogicalAddress) error {
	if !address.IsValid() {
		return ErrInvalidLogicalAddress
	}
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()

	if C.libcec_power_on_devices(c.handle, C.cec_logical_address(address)) == 0 {
		return fmt.Errorf("%w: power on %d", ErrLibcecCall, address)
	}
	return nil
}

// Standby puts a device in standby mode. address must be 0..14.
func (c *Connection) Standby(address LogicalAddress) error {
	if !address.IsValid() {
		return ErrInvalidLogicalAddress
	}
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()

	if C.libcec_standby_devices(c.handle, C.cec_logical_address(address)) == 0 {
		return fmt.Errorf("%w: standby %d", ErrLibcecCall, address)
	}
	return nil
}

// SetActiveSource declares the local device of the given type to be active.
func (c *Connection) SetActiveSource(deviceType DeviceType) error {
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	if C.libcec_set_active_source(c.handle, C.cec_device_type(deviceType)) == 0 {
		return fmt.Errorf("%w: set active source", ErrLibcecCall)
	}
	return nil
}

// SetInactiveView marks the local device as inactive view.
func (c *Connection) SetInactiveView() error {
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	if C.libcec_set_inactive_view(c.handle) == 0 {
		return fmt.Errorf("%w: set inactive view", ErrLibcecCall)
	}
	return nil
}

// VolumeUp increases the system audio volume.
func (c *Connection) VolumeUp(sendRelease bool) error {
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	rel := C.int(0)
	if sendRelease {
		rel = 1
	}
	if C.libcec_volume_up(c.handle, rel) == 0 {
		return fmt.Errorf("%w: volume up", ErrLibcecCall)
	}
	return nil
}

// VolumeDown decreases the system audio volume.
func (c *Connection) VolumeDown(sendRelease bool) error {
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	rel := C.int(0)
	if sendRelease {
		rel = 1
	}
	if C.libcec_volume_down(c.handle, rel) == 0 {
		return fmt.Errorf("%w: volume down", ErrLibcecCall)
	}
	return nil
}

// AudioToggleMute toggles the audio mute state.
func (c *Connection) AudioToggleMute() error {
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	if C.libcec_audio_toggle_mute(c.handle) == 0 {
		return fmt.Errorf("%w: audio toggle mute", ErrLibcecCall)
	}
	return nil
}

// AudioMute mutes audio.
func (c *Connection) AudioMute() error {
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	if C.libcec_audio_mute(c.handle) == 0 {
		return fmt.Errorf("%w: audio mute", ErrLibcecCall)
	}
	return nil
}

// AudioUnmute unmutes audio.
func (c *Connection) AudioUnmute() error {
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	if C.libcec_audio_unmute(c.handle) == 0 {
		return fmt.Errorf("%w: audio unmute", ErrLibcecCall)
	}
	return nil
}

// GetDevicePowerStatus queries a device's power status.
// Returns PowerStatusUnknown with a wrapped ErrLibcecCall when the bus does
// not respond.
func (c *Connection) GetDevicePowerStatus(address LogicalAddress) (PowerStatus, error) {
	if !address.IsValid() {
		return PowerStatusUnknown, ErrInvalidLogicalAddress
	}
	if err := c.guard(); err != nil {
		return PowerStatusUnknown, err
	}
	defer c.apiMu.Unlock()
	status := C.libcec_get_device_power_status(c.handle, C.cec_logical_address(address))
	if status == C.CEC_POWER_STATUS_UNKNOWN {
		return PowerStatusUnknown, fmt.Errorf("%w: get power status %d", ErrLibcecCall, address)
	}
	return PowerStatus(status), nil
}

// GetActiveSource returns the logical address that currently claims the
// active-source role. Returns ErrNoActiveSource if no device claims it.
func (c *Connection) GetActiveSource() (LogicalAddress, error) {
	if err := c.guard(); err != nil {
		return LogicalAddressUnknown, err
	}
	defer c.apiMu.Unlock()
	addr := C.libcec_get_active_source(c.handle)
	la := LogicalAddress(uint8(addr))
	if !la.IsValid() {
		return la, ErrNoActiveSource
	}
	return la, nil
}

// IsActiveSource reports whether the given device is the current active source.
func (c *Connection) IsActiveSource(address LogicalAddress) bool {
	if !address.IsValid() {
		return false
	}
	if err := c.guard(); err != nil {
		return false
	}
	defer c.apiMu.Unlock()
	return C.libcec_is_active_source(c.handle, C.cec_logical_address(address)) == 1
}

// GetDeviceVendorId queries a device's vendor ID. Returns 0 + wrapped error
// when the device does not respond.
func (c *Connection) GetDeviceVendorId(address LogicalAddress) (uint64, error) {
	if !address.IsValid() {
		return 0, ErrInvalidLogicalAddress
	}
	if err := c.guard(); err != nil {
		return 0, err
	}
	defer c.apiMu.Unlock()
	v := C.libcec_get_device_vendor_id(c.handle, C.cec_logical_address(address))
	if v == C.CEC_VENDOR_UNKNOWN {
		return 0, fmt.Errorf("%w: vendor id %d", ErrLibcecCall, address)
	}
	return uint64(v), nil
}

// GetDevicePhysicalAddress queries a device's physical (HDMI tree) address.
func (c *Connection) GetDevicePhysicalAddress(address LogicalAddress) (uint16, error) {
	if !address.IsValid() {
		return 0, ErrInvalidLogicalAddress
	}
	if err := c.guard(); err != nil {
		return 0, err
	}
	defer c.apiMu.Unlock()
	a := C.libcec_get_device_physical_address(c.handle, C.cec_logical_address(address))
	if a == C.CEC_INVALID_PHYSICAL_ADDRESS {
		return 0, fmt.Errorf("%w: physical address %d", ErrLibcecCall, address)
	}
	return uint16(a), nil
}

// GetDeviceOSDName queries a device's OSD name.
func (c *Connection) GetDeviceOSDName(address LogicalAddress) (string, error) {
	if !address.IsValid() {
		return "", ErrInvalidLogicalAddress
	}
	if err := c.guard(); err != nil {
		return "", err
	}
	defer c.apiMu.Unlock()
	var name [14]C.char
	if C.libcec_get_device_osd_name(c.handle, C.cec_logical_address(address), &name[0]) == 0 {
		return "", fmt.Errorf("%w: osd name %d", ErrLibcecCall, address)
	}
	return C.GoString(&name[0]), nil
}

// GetDeviceMenuLanguage queries a device's menu language (ISO 639-2).
func (c *Connection) GetDeviceMenuLanguage(address LogicalAddress) (string, error) {
	if !address.IsValid() {
		return "", ErrInvalidLogicalAddress
	}
	if err := c.guard(); err != nil {
		return "", err
	}
	defer c.apiMu.Unlock()
	var lang [4]C.char
	if C.libcec_get_device_menu_language(c.handle, C.cec_logical_address(address), &lang[0]) == 0 {
		return "", fmt.Errorf("%w: menu lang %d", ErrLibcecCall, address)
	}
	return C.GoString(&lang[0]), nil
}

// GetDeviceCecVersion queries a device's CEC spec version.
func (c *Connection) GetDeviceCecVersion(address LogicalAddress) (CECVersion, error) {
	if !address.IsValid() {
		return CECVersionUnknown, ErrInvalidLogicalAddress
	}
	if err := c.guard(); err != nil {
		return CECVersionUnknown, err
	}
	defer c.apiMu.Unlock()
	v := C.libcec_get_device_cec_version(c.handle, C.cec_logical_address(address))
	if v == C.CEC_VERSION_UNKNOWN {
		return CECVersionUnknown, fmt.Errorf("%w: cec version %d", ErrLibcecCall, address)
	}
	return CECVersion(v), nil
}

// GetActiveDevices returns the logical addresses of devices that libcec
// considers "active" on the bus.
func (c *Connection) GetActiveDevices() []LogicalAddress {
	if err := c.guard(); err != nil {
		return nil
	}
	defer c.apiMu.Unlock()
	addrs := C.libcec_get_active_devices(c.handle)
	out := make([]LogicalAddress, 0, 16)
	for i := 0; i < 16; i++ {
		if addrs.addresses[i] != 0 {
			out = append(out, LogicalAddress(i))
		}
	}
	return out
}

// IsActiveDevice reports whether libcec considers the given device active.
func (c *Connection) IsActiveDevice(address LogicalAddress) bool {
	if !address.IsValid() {
		return false
	}
	if err := c.guard(); err != nil {
		return false
	}
	defer c.apiMu.Unlock()
	return C.libcec_is_active_device(c.handle, C.cec_logical_address(address)) == 1
}

// Transmit sends a raw CEC command frame.
func (c *Connection) Transmit(cmd *Command) error {
	if cmd == nil {
		return fmt.Errorf("%w: nil command", ErrTransmitFailed)
	}
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()

	cCmd := C.cec_command{}
	cCmd.initiator = C.cec_logical_address(cmd.Initiator)
	cCmd.destination = C.cec_logical_address(cmd.Destination)
	cCmd.opcode = C.cec_opcode(cmd.Opcode)
	if cmd.OpcodeSet {
		cCmd.opcode_set = 1
	}
	cCmd.parameters.size = C.uint8_t(len(cmd.Parameters))
	for i, p := range cmd.Parameters {
		cCmd.parameters.data[i] = C.uint8_t(p)
	}
	if C.libcec_transmit(c.handle, &cCmd) == 0 {
		return ErrTransmitFailed
	}
	return nil
}

// SendKeypress sends a remote-control key press to a device.
// If wait is true, the call blocks until the bus acknowledges.
func (c *Connection) SendKeypress(address LogicalAddress, key Keycode, wait bool) error {
	if !address.IsValid() {
		return ErrInvalidLogicalAddress
	}
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	w := C.int(0)
	if wait {
		w = 1
	}
	if C.libcec_send_keypress(c.handle, C.cec_logical_address(address),
		C.cec_user_control_code(key), w) == 0 {
		return fmt.Errorf("%w: send keypress %d", ErrLibcecCall, address)
	}
	return nil
}

// SendKeyRelease sends a remote-control key release to a device.
func (c *Connection) SendKeyRelease(address LogicalAddress, wait bool) error {
	if !address.IsValid() {
		return ErrInvalidLogicalAddress
	}
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	w := C.int(0)
	if wait {
		w = 1
	}
	if C.libcec_send_key_release(c.handle, C.cec_logical_address(address), w) == 0 {
		return fmt.Errorf("%w: send key release %d", ErrLibcecCall, address)
	}
	return nil
}

// SetOSDString displays an OSD string on the given device.
func (c *Connection) SetOSDString(address LogicalAddress, duration DisplayControl, message string) error {
	if !address.IsValid() {
		return ErrInvalidLogicalAddress
	}
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	cMsg := C.CString(message)
	defer C.free(unsafe.Pointer(cMsg))
	if C.libcec_set_osd_string(c.handle, C.cec_logical_address(address),
		C.cec_display_control(duration), cMsg) == 0 {
		return fmt.Errorf("%w: set osd string", ErrLibcecCall)
	}
	return nil
}

// SwitchMonitoring toggles libcec monitoring mode.
func (c *Connection) SwitchMonitoring(enable bool) error {
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	v := C.int(0)
	if enable {
		v = 1
	}
	if C.libcec_switch_monitoring(c.handle, v) == 0 {
		return fmt.Errorf("%w: switch monitoring", ErrLibcecCall)
	}
	return nil
}

// GetLibInfo returns libcec version information as a printable string.
func (c *Connection) GetLibInfo() string {
	if err := c.guard(); err != nil {
		return ""
	}
	defer c.apiMu.Unlock()
	return C.GoString(C.libcec_get_lib_info(c.handle))
}

// SetConfiguration replaces the running libcec configuration. It re-attaches
// the cec package's internal callback table so events keep flowing after the
// swap. cfg.DeviceName, DeviceType, PhysicalAddress, BaseDevice, HDMIPort and
// ClientVersion are honored.
func (c *Connection) SetConfiguration(cfg *Configuration) error {
	if cfg == nil {
		return fmt.Errorf("cec: nil Configuration")
	}
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()

	cConfig := C.libcec_configuration{}
	C.libcec_clear_configuration(&cConfig)

	cName := C.CString(cfg.DeviceName)
	defer C.free(unsafe.Pointer(cName))
	C.strncpy(&cConfig.strDeviceName[0], cName, C.LIBCEC_OSD_NAME_SIZE-1)

	cConfig.deviceTypes.types[0] = C.cec_device_type(cfg.DeviceType)
	cConfig.iPhysicalAddress = C.uint16_t(cfg.PhysicalAddress)
	cConfig.baseDevice = C.cec_logical_address(cfg.BaseDevice)
	cConfig.iHDMIPort = C.uint8_t(cfg.HDMIPort)
	cConfig.clientVersion = C.uint32_t(cfg.ClientVersion)

	C.cec_install_callbacks(&cConfig, C.uintptr_t(c.cgoHandle))

	if C.libcec_set_configuration(c.handle, &cConfig) == 0 {
		return fmt.Errorf("%w: set configuration", ErrLibcecCall)
	}
	c.config = cfg
	return nil
}

// GetCurrentConfiguration retrieves the running libcec configuration.
func (c *Connection) GetCurrentConfiguration() (*Configuration, error) {
	if err := c.guard(); err != nil {
		return nil, err
	}
	defer c.apiMu.Unlock()
	var cConfig C.libcec_configuration
	if C.libcec_get_current_configuration(c.handle, &cConfig) == 0 {
		return nil, fmt.Errorf("%w: get current configuration", ErrLibcecCall)
	}
	return &Configuration{
		DeviceName:      C.GoString(&cConfig.strDeviceName[0]),
		DeviceType:      DeviceType(cConfig.deviceTypes.types[0]),
		PhysicalAddress: uint16(cConfig.iPhysicalAddress),
		BaseDevice:      LogicalAddress(cConfig.baseDevice),
		HDMIPort:        uint8(cConfig.iHDMIPort),
		ClientVersion:   uint32(cConfig.clientVersion),
		ServerVersion:   uint32(cConfig.serverVersion),
	}, nil
}

// GetAudioStatus returns the current audio status from the audio system.
// rawStatus is the full byte from libcec_audio_get_status (bit7 = muted,
// bits0-6 = level). Volume may exceed 100 when no audio system is present.
func (c *Connection) GetAudioStatus() (volume uint8, muted bool, rawStatus uint8) {
	if err := c.guard(); err != nil {
		return 0, false, 0
	}
	defer c.apiMu.Unlock()
	status := C.libcec_audio_get_status(c.handle)
	rawStatus = uint8(status)
	muted = (status & 0x80) != 0
	volume = uint8(status & 0x7F)
	return volume, muted, rawStatus
}

// PollDevice sends a CEC POLL message to test whether a device is present.
// This is much faster than a full GetDeviceInfo and does not require an OSD
// name response.
func (c *Connection) PollDevice(address LogicalAddress) bool {
	if !address.IsValid() {
		return false
	}
	if err := c.guard(); err != nil {
		return false
	}
	defer c.apiMu.Unlock()
	return C.libcec_poll_device(c.handle, C.cec_logical_address(address)) == 1
}

// SetHDMIPort tells libcec to switch input on the base device to the given
// HDMI port. baseDevice is typically LogicalAddressTV (0).
func (c *Connection) SetHDMIPort(baseDevice LogicalAddress, port uint8) error {
	if port < 1 || port > 15 {
		return ErrInvalidHDMIPort
	}
	if err := c.guard(); err != nil {
		return err
	}
	defer c.apiMu.Unlock()
	if C.libcec_set_hdmi_port(c.handle, C.cec_logical_address(baseDevice), C.uint8_t(port)) == 0 {
		return fmt.Errorf("%w: set hdmi port %d", ErrLibcecCall, port)
	}
	return nil
}

// RescanDevices asks libcec to re-discover devices on the bus. This call does
// not sleep; pass settle if you need to wait for responses to arrive
// (typical: 1-2s for cold buses, 0 for hot buses where you'll observe the
// updates via Events).
func (c *Connection) RescanDevices(settle time.Duration) error {
	if err := c.guard(); err != nil {
		return err
	}
	C.libcec_rescan_devices(c.handle)
	c.apiMu.Unlock()

	if settle > 0 {
		// Settle outside the lock so other libcec calls can proceed.
		t := time.NewTimer(settle)
		defer t.Stop()
		<-t.C
	}
	return nil
}

// GetLogicalAddresses returns all logical addresses currently assigned to
// this adapter. If libcec reports an empty bitmask but a primary address is
// known, that single primary address is returned.
func (c *Connection) GetLogicalAddresses() []LogicalAddress {
	if err := c.guard(); err != nil {
		return nil
	}
	defer c.apiMu.Unlock()
	addrs := C.libcec_get_logical_addresses(c.handle)
	out := make([]LogicalAddress, 0, 16)
	for i := 0; i < 16; i++ {
		if addrs.addresses[i] != 0 {
			out = append(out, LogicalAddress(i))
		}
	}
	if len(out) == 0 && addrs.primary != C.CECDEVICE_UNKNOWN {
		out = append(out, LogicalAddress(uint8(addrs.primary)))
	}
	return out
}

// pingTV is the cheapest health check we can do; used by reconnect supervisors.
// It returns ErrClosed or ErrLibcecCall on failure.
func (c *Connection) pingTV(ctx context.Context) error {
	_ = ctx
	_, err := c.GetDevicePowerStatus(LogicalAddressTV)
	return err
}
