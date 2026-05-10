package cec

import (
	"context"
	"errors"
	"fmt"
	"sort"
	"strings"
	"time"
)

// DeviceInfoErrors collects the per-field errors encountered while building
// a Device with GetDeviceInfo. Use errors.As to inspect from the returned
// error from GetDeviceInfo.
type DeviceInfoErrors struct {
	PhysicalAddress error
	VendorID        error
	CECVersion      error
	PowerStatus     error
	OSDName         error
	MenuLanguage    error
}

// Any reports whether at least one sub-query failed.
func (d DeviceInfoErrors) Any() bool {
	return d.PhysicalAddress != nil || d.VendorID != nil || d.CECVersion != nil ||
		d.PowerStatus != nil || d.OSDName != nil || d.MenuLanguage != nil
}

// All reports whether every sub-query failed.
func (d DeviceInfoErrors) All() bool {
	return d.PhysicalAddress != nil && d.VendorID != nil && d.CECVersion != nil &&
		d.PowerStatus != nil && d.OSDName != nil && d.MenuLanguage != nil
}

func (d DeviceInfoErrors) Error() string {
	parts := []string{}
	if d.PhysicalAddress != nil {
		parts = append(parts, "physical:"+d.PhysicalAddress.Error())
	}
	if d.VendorID != nil {
		parts = append(parts, "vendor:"+d.VendorID.Error())
	}
	if d.CECVersion != nil {
		parts = append(parts, "cec_version:"+d.CECVersion.Error())
	}
	if d.PowerStatus != nil {
		parts = append(parts, "power:"+d.PowerStatus.Error())
	}
	if d.OSDName != nil {
		parts = append(parts, "osd:"+d.OSDName.Error())
	}
	if d.MenuLanguage != nil {
		parts = append(parts, "menu_lang:"+d.MenuLanguage.Error())
	}
	if len(parts) == 0 {
		return "cec: device info: <no errors>"
	}
	return "cec: device info: " + strings.Join(parts, ", ")
}

// GetDeviceInfo retrieves comprehensive information about a device. The
// returned Device is always non-nil; missing fields keep their zero value.
// The returned error is non-nil iff every sub-query failed (i.e. the device
// is unresponsive). Partial failures are surfaced via DeviceInfoErrors which
// callers may inspect with errors.As.
func (c *Connection) GetDeviceInfo(address LogicalAddress) (*Device, error) {
	dev := &Device{
		LogicalAddress: address,
		IsActive:       c.IsActiveDevice(address),
		IsActiveSource: c.IsActiveSource(address),
	}
	var errs DeviceInfoErrors

	if v, err := c.GetDevicePhysicalAddress(address); err == nil {
		dev.PhysicalAddress = v
	} else {
		errs.PhysicalAddress = err
	}
	if v, err := c.GetDeviceVendorId(address); err == nil {
		dev.VendorID = v
	} else {
		errs.VendorID = err
	}
	if v, err := c.GetDeviceCecVersion(address); err == nil {
		dev.CECVersion = v
	} else {
		errs.CECVersion = err
	}
	if v, err := c.GetDevicePowerStatus(address); err == nil {
		dev.PowerStatus = v
	} else {
		errs.PowerStatus = err
	}
	if v, err := c.GetDeviceOSDName(address); err == nil {
		dev.OSDName = v
	} else {
		errs.OSDName = err
	}
	if v, err := c.GetDeviceMenuLanguage(address); err == nil {
		dev.MenuLanguage = v
	} else {
		errs.MenuLanguage = err
	}

	if errs.All() {
		return dev, errs
	}
	return dev, nil
}

// GetAllDevices triggers a bus rescan and then returns a Device for every
// libcec-active address. The settle delay is passed through to RescanDevices;
// pass 0 to skip waiting (useful when you'll observe updates via Events).
func (c *Connection) GetAllDevices(settle time.Duration) ([]*Device, error) {
	if err := c.RescanDevices(settle); err != nil {
		return nil, err
	}
	return c.GetAllDevicesNoRescan(), nil
}

// GetAllDevicesNoRescan returns a Device for each libcec-active address
// without triggering a rescan. Suitable for frequent UI refreshes.
func (c *Connection) GetAllDevicesNoRescan() []*Device {
	addrs := c.GetActiveDevices()
	out := make([]*Device, 0, len(addrs))
	for _, a := range addrs {
		dev, err := c.GetDeviceInfo(a)
		if err != nil {
			// Even an "all-failed" device is still discoverable, but typical
			// callers want to know about live devices only - skip silently.
			continue
		}
		out = append(out, dev)
	}
	return out
}

// WaitForDeviceReady polls a device's power status until it matches target,
// the context is cancelled, or the per-iteration timeout elapses. It uses
// context cancellation rather than a hard wall-clock deadline so callers can
// abort cleanly on shutdown.
func (c *Connection) WaitForDeviceReady(ctx context.Context, address LogicalAddress, target PowerStatus, poll time.Duration) error {
	if poll <= 0 {
		poll = 500 * time.Millisecond
	}
	if !address.IsValid() {
		return ErrInvalidLogicalAddress
	}
	t := time.NewTicker(poll)
	defer t.Stop()
	for {
		status, err := c.GetDevicePowerStatus(address)
		if err == nil && status == target {
			return nil
		}
		if errors.Is(err, ErrClosed) {
			return err
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-t.C:
		}
	}
}

// ownAddress returns the adapter's primary logical address, falling back to
// LogicalAddressFreeUse if the adapter has not registered any.
func (c *Connection) ownAddress() LogicalAddress {
	addrs := c.GetLogicalAddresses()
	if len(addrs) > 0 {
		return addrs[0]
	}
	return LogicalAddressFreeUse
}

// SendImageViewOn sends Image View On (0x04) to the TV to wake it before
// switching sources.
func (c *Connection) SendImageViewOn() error {
	return c.Transmit(&Command{
		Initiator:   c.ownAddress(),
		Destination: LogicalAddressTV,
		Opcode:      OpcodeImageViewOn,
		OpcodeSet:   true,
	})
}

// SwitchToHDMIPort switches the TV input to the given HDMI port. It first
// nudges the TV awake with Image View On, then uses libcec's SetHDMIPort
// (preferred), falling back to an Active Source broadcast.
func (c *Connection) SwitchToHDMIPort(port uint8) error {
	if port < 1 || port > 15 {
		return ErrInvalidHDMIPort
	}
	_ = c.SendImageViewOn()
	time.Sleep(300 * time.Millisecond)

	if err := c.SetHDMIPort(LogicalAddressTV, port); err == nil {
		return nil
	}
	phys := uint16(port) << 12
	return c.Transmit(&Command{
		Initiator:   c.ownAddress(),
		Destination: LogicalAddressBroadcast,
		Opcode:      OpcodeActiveSource,
		OpcodeSet:   true,
		Parameters:  []uint8{uint8(phys >> 8), uint8(phys & 0xFF)},
	})
}

// SwitchToDevice broadcasts Active Source for the given device's physical
// address, prompting the TV to switch input to that device's HDMI port.
func (c *Connection) SwitchToDevice(address LogicalAddress) error {
	if !address.IsValid() {
		return ErrInvalidLogicalAddress
	}
	_ = c.SendImageViewOn()
	time.Sleep(300 * time.Millisecond)

	phys, err := c.GetDevicePhysicalAddress(address)
	if err != nil {
		return fmt.Errorf("switch to %d: %w", address, err)
	}
	return c.Transmit(&Command{
		Initiator:   c.ownAddress(),
		Destination: LogicalAddressBroadcast,
		Opcode:      OpcodeActiveSource,
		OpcodeSet:   true,
		Parameters:  []uint8{uint8(phys >> 8), uint8(phys & 0xFF)},
	})
}

// SendVolumeKey sends a volume keypress directly to a specific device,
// holding it long enough to be registered, then releases.
func (c *Connection) SendVolumeKey(address LogicalAddress, key Keycode) error {
	if err := c.SendKeypress(address, key, true); err != nil {
		return err
	}
	time.Sleep(300 * time.Millisecond)
	return c.SendKeyRelease(address, true)
}

// nudgeSetSystemAudioMode hints to the TV that system-audio mode should be
// on. Many AVRs only forward volume after this. Errors are ignored - many
// TVs feature-abort.
func (c *Connection) nudgeSetSystemAudioMode(on bool) {
	own := c.ownAddress()
	b := byte(0)
	if on {
		b = 1
	}
	_ = c.Transmit(&Command{
		Initiator:   own,
		Destination: LogicalAddressTV,
		Opcode:      OpcodeSetSystemAudioMode,
		OpcodeSet:   true,
		Parameters:  []uint8{b},
	})
}

// volumePassThroughOrder returns the logical-address order to try for
// best-effort volume routing.
func volumePassThroughOrder() []LogicalAddress {
	return []LogicalAddress{
		LogicalAddressAudioSystem,
		LogicalAddressTV,
		LogicalAddressPlaybackDevice1,
		LogicalAddressPlaybackDevice2,
	}
}

// VolumeUpBestEffort tries CEC user-control volume on common destinations,
// then falls back to libcec_volume_up.
func (c *Connection) VolumeUpBestEffort(sendRelease bool) error {
	c.nudgeSetSystemAudioMode(true)
	time.Sleep(80 * time.Millisecond)
	for _, dest := range volumePassThroughOrder() {
		if dest == c.ownAddress() {
			continue
		}
		if err := c.SendVolumeKey(dest, KeycodeVolumeUp); err == nil {
			return nil
		}
	}
	return c.VolumeUp(sendRelease)
}

// VolumeDownBestEffort mirrors VolumeUpBestEffort.
func (c *Connection) VolumeDownBestEffort(sendRelease bool) error {
	c.nudgeSetSystemAudioMode(true)
	time.Sleep(80 * time.Millisecond)
	for _, dest := range volumePassThroughOrder() {
		if dest == c.ownAddress() {
			continue
		}
		if err := c.SendVolumeKey(dest, KeycodeVolumeDown); err == nil {
			return nil
		}
	}
	return c.VolumeDown(sendRelease)
}

// MuteBestEffort tries user-control mute on common targets, then libcec toggle.
func (c *Connection) MuteBestEffort() error {
	c.nudgeSetSystemAudioMode(true)
	time.Sleep(80 * time.Millisecond)
	for _, dest := range volumePassThroughOrder() {
		if dest == c.ownAddress() {
			continue
		}
		if err := c.SendVolumeKey(dest, KeycodeMute); err == nil {
			return nil
		}
	}
	return c.AudioToggleMute()
}

// LogicalAddressesWithOptionalPoll returns libcec's active logical addresses
// plus any additional addresses that respond to a CEC POLL but were not in
// the active mask. fullPoll probes every missing address 0..14; otherwise it
// probes the well-known role addresses only.
func (c *Connection) LogicalAddressesWithOptionalPoll(fullPoll bool) []LogicalAddress {
	active := c.GetActiveDevices()
	seen := make(map[LogicalAddress]struct{}, len(active)+16)
	for _, a := range active {
		seen[a] = struct{}{}
	}
	out := make([]LogicalAddress, 0, len(active)+8)
	out = append(out, active...)

	var probeOrder []LogicalAddress
	if fullPoll {
		for a := LogicalAddress(0); a <= 14; a++ {
			probeOrder = append(probeOrder, a)
		}
	} else {
		probeOrder = []LogicalAddress{
			LogicalAddressTV,
			LogicalAddressRecordingDevice1,
			LogicalAddressRecordingDevice2,
			LogicalAddressTuner1,
			LogicalAddressPlaybackDevice1,
			LogicalAddressAudioSystem,
			LogicalAddressTuner2,
			LogicalAddressTuner3,
			LogicalAddressPlaybackDevice2,
			LogicalAddressRecordingDevice3,
			LogicalAddressTuner4,
			LogicalAddressPlaybackDevice3,
		}
	}

	for _, a := range probeOrder {
		if _, ok := seen[a]; ok {
			continue
		}
		if c.PollDevice(a) {
			seen[a] = struct{}{}
			out = append(out, a)
		}
	}

	sort.Slice(out, func(i, j int) bool { return out[i] < out[j] })
	return out
}

// DeviceTypeForAddress returns the expected DeviceType for a logical address.
func DeviceTypeForAddress(addr LogicalAddress) DeviceType {
	switch addr {
	case LogicalAddressTV:
		return DeviceTypeTV
	case LogicalAddressRecordingDevice1, LogicalAddressRecordingDevice2, LogicalAddressRecordingDevice3:
		return DeviceTypeRecordingDevice
	case LogicalAddressTuner1, LogicalAddressTuner2, LogicalAddressTuner3, LogicalAddressTuner4:
		return DeviceTypeTuner
	case LogicalAddressPlaybackDevice1, LogicalAddressPlaybackDevice2, LogicalAddressPlaybackDevice3:
		return DeviceTypePlaybackDevice
	case LogicalAddressAudioSystem:
		return DeviceTypeAudioSystem
	default:
		return DeviceTypeReserved
	}
}

// vendorNames is the static vendor-ID -> human-name lookup, built once at
// init time so GetVendorName allocates nothing on the hot path.
var vendorNames = map[uint64]string{
	0x000039: "Toshiba",
	0x0000F0: "Samsung",
	0x0005CD: "Denon",
	0x000678: "Marantz",
	0x000982: "Loewe",
	0x0009B0: "Onkyo",
	0x000CB8: "Medion",
	0x000CE7: "Toshiba",
	0x001582: "Pulse Eight",
	0x001950: "Google",
	0x001A11: "Akai",
	0x0020C7: "AOC",
	0x002467: "Panasonic",
	0x008045: "Philips",
	0x00903E: "Pioneer",
	0x009053: "LG",
	0x00A0DE: "Sharp",
	0x00D0D5: "Vizio",
	0x00E036: "Harman Kardon",
	0x00E091: "Yamaha",
	0x08001F: "Sony",
	0x18C086: "Broadcom",
	0x6B746D: "Vizio",
	0x8065E9: "Benq",
	0x9C645E: "Daewoo",
}

// GetVendorName returns a human-readable vendor name for a CEC vendor ID.
// Unknown IDs are formatted as "Unknown (0xABCDEF)".
func GetVendorName(vendorId uint64) string {
	if name, ok := vendorNames[vendorId]; ok {
		return name
	}
	return fmt.Sprintf("Unknown (0x%06X)", vendorId)
}

// PhysicalAddressToString converts a packed physical address into dotted form
// (e.g. 0x2100 -> "2.1.0.0").
func PhysicalAddressToString(addr uint16) string {
	a := (addr >> 12) & 0xF
	b := (addr >> 8) & 0xF
	c := (addr >> 4) & 0xF
	d := addr & 0xF
	return fmt.Sprintf("%d.%d.%d.%d", a, b, c, d)
}

// ParsePhysicalAddress parses dotted form back into the packed uint16.
func ParsePhysicalAddress(addrStr string) (uint16, error) {
	parts := strings.Split(addrStr, ".")
	if len(parts) != 4 {
		return 0, fmt.Errorf("cec: physical address %q must have 4 dotted components", addrStr)
	}
	out := uint16(0)
	for i, p := range parts {
		var v uint16
		n, err := fmt.Sscanf(p, "%d", &v)
		if err != nil || n != 1 {
			return 0, fmt.Errorf("cec: physical address %q: bad component %d: %v", addrStr, i+1, err)
		}
		if v > 15 {
			return 0, fmt.Errorf("cec: physical address components must be 0-15 (got %d in %q)", v, addrStr)
		}
		out = (out << 4) | v
	}
	return out, nil
}

// SendButton sends a brief key press + release.
func (c *Connection) SendButton(address LogicalAddress, key Keycode) error {
	if err := c.SendKeypress(address, key, false); err != nil {
		return err
	}
	time.Sleep(100 * time.Millisecond)
	return c.SendKeyRelease(address, false)
}

// NavigateMenu is a convenience wrapper around SendButton for menu navigation.
func (c *Connection) NavigateMenu(address LogicalAddress, direction Keycode) error {
	return c.SendButton(address, direction)
}

// SetVolume sends repeated VolumeUp/VolumeDown to step from currentLevel to
// targetLevel. The poll spacing matches what most AVRs accept reliably.
func (c *Connection) SetVolume(targetLevel, currentLevel int) error {
	if targetLevel == currentLevel {
		return nil
	}
	steps := targetLevel - currentLevel
	if steps < 0 {
		steps = -steps
	}
	for i := 0; i < steps; i++ {
		var err error
		if targetLevel > currentLevel {
			err = c.VolumeUp(true)
		} else {
			err = c.VolumeDown(true)
		}
		if err != nil {
			return err
		}
		time.Sleep(100 * time.Millisecond)
	}
	return nil
}
