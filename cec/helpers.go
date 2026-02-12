package cec

import (
	"fmt"
	"time"
)

// Helper functions for common operations

// GetDeviceInfo retrieves comprehensive information about a device
func (c *Connection) GetDeviceInfo(address LogicalAddress) (*Device, error) {
	device := &Device{
		LogicalAddress: address,
		IsActive:       c.IsActiveDevice(address),
		IsActiveSource: c.IsActiveSource(address),
	}

	// Get physical address
	if physAddr, err := c.GetDevicePhysicalAddress(address); err == nil {
		device.PhysicalAddress = physAddr
	}

	// Get vendor ID
	if vendorId, err := c.GetDeviceVendorId(address); err == nil {
		device.VendorID = vendorId
	}

	// Get CEC version
	if version, err := c.GetDeviceCecVersion(address); err == nil {
		device.CECVersion = version
	}

	// Get power status
	if power, err := c.GetDevicePowerStatus(address); err == nil {
		device.PowerStatus = power
	}

	// Get OSD name
	if name, err := c.GetDeviceOSDName(address); err == nil {
		device.OSDName = name
	}

	// Get menu language
	if lang, err := c.GetDeviceMenuLanguage(address); err == nil {
		device.MenuLanguage = lang
	}

	return device, nil
}

// GetAllDevices scans and returns information about all active devices
func (c *Connection) GetAllDevices() ([]*Device, error) {
	// Rescan to ensure we have latest device info
	if err := c.RescanDevices(); err != nil {
		return nil, err
	}

	return c.GetAllDevicesNoRescan()
}

// GetAllDevicesNoRescan returns information about all active devices without
// triggering a bus rescan. This is useful for frequent queries where a full
// rescan would be too expensive.
func (c *Connection) GetAllDevicesNoRescan() ([]*Device, error) {
	addresses := c.GetActiveDevices()
	devices := make([]*Device, 0, len(addresses))

	for _, addr := range addresses {
		device, err := c.GetDeviceInfo(addr)
		if err == nil {
			devices = append(devices, device)
		}
	}

	return devices, nil
}

// WaitForDeviceReady waits for a device to reach a specific power state
func (c *Connection) WaitForDeviceReady(address LogicalAddress, targetState PowerStatus, timeout time.Duration) error {
	deadline := time.Now().Add(timeout)

	for time.Now().Before(deadline) {
		status, err := c.GetDevicePowerStatus(address)
		if err == nil && status == targetState {
			return nil
		}
		time.Sleep(500 * time.Millisecond)
	}

	return fmt.Errorf("timeout waiting for device %d to reach state %v", address, targetState)
}

// getOwnAddress returns the adapter's own logical address on the CEC bus.
func (c *Connection) getOwnAddress() LogicalAddress {
	addrs := c.GetLogicalAddresses()
	if len(addrs) > 0 {
		return addrs[0]
	}
	// Fallback: use unregistered/free-use address so transmit doesn't fail
	// due to an address the adapter doesn't own.
	return LogicalAddressFreeUse
}

// sendImageViewOn sends Image View On (0x04) to the TV to wake it up and
// ensure it is ready to process source-switching commands.
func (c *Connection) sendImageViewOn() error {
	cmd := &Command{
		Initiator:   c.getOwnAddress(),
		Destination: LogicalAddressTV,
		Opcode:      OpcodeImageViewOn,
		OpcodeSet:   true,
	}
	return c.Transmit(cmd)
}

// SwitchToHDMIPort switches TV input to a specific HDMI port.
// Uses libcec's built-in SetHDMIPort as the primary method (which handles
// CEC protocol correctly), with an Active Source broadcast as fallback.
func (c *Connection) SwitchToHDMIPort(port uint8) error {
	if port < 1 || port > 15 {
		return fmt.Errorf("invalid HDMI port %d (must be 1-15)", port)
	}

	// Wake up the TV first so it processes the source switch
	c.sendImageViewOn()
	time.Sleep(300 * time.Millisecond)

	// Primary: use libcec's built-in HDMI port switching
	if err := c.SetHDMIPort(LogicalAddressTV, port); err == nil {
		return nil
	}

	// Fallback: send Active Source broadcast with the port's physical address
	physicalAddress := uint16(port) << 12
	cmd := &Command{
		Initiator:   c.getOwnAddress(),
		Destination: LogicalAddressBroadcast,
		Opcode:      OpcodeActiveSource,
		OpcodeSet:   true,
		Parameters: []uint8{
			uint8(physicalAddress >> 8),
			uint8(physicalAddress & 0xFF),
		},
	}

	return c.Transmit(cmd)
}

// SwitchToDevice switches to a specific device by its logical address
func (c *Connection) SwitchToDevice(address LogicalAddress) error {
	// Wake up the TV so it is ready to process the source switch
	c.sendImageViewOn()
	time.Sleep(300 * time.Millisecond)

	// Get device's physical address
	physAddr, err := c.GetDevicePhysicalAddress(address)
	if err != nil {
		return fmt.Errorf("failed to get physical address: %w", err)
	}

	// Send Active Source broadcast with the target device's physical address.
	// TVs respond to Active Source by switching to the corresponding input.
	cmd := &Command{
		Initiator:   c.getOwnAddress(),
		Destination: LogicalAddressBroadcast,
		Opcode:      OpcodeActiveSource,
		OpcodeSet:   true,
		Parameters: []uint8{
			uint8(physAddr >> 8),
			uint8(physAddr & 0xFF),
		},
	}

	return c.Transmit(cmd)
}

// SendVolumeKey sends a volume key press directly to a specific device address.
// Uses wait=true so libcec waits for bus acknowledgment, and a longer hold
// time so the target device registers the key press.
func (c *Connection) SendVolumeKey(address LogicalAddress, key Keycode) error {
	// Send press with wait=true for ACK
	if err := c.SendKeypress(address, key, true); err != nil {
		return err
	}

	// Hold the key long enough for the device to register it
	time.Sleep(300 * time.Millisecond)

	// Release with wait=true
	return c.SendKeyRelease(address, true)
}

// PortInfo describes one HDMI port on the display and which devices are on it.
type PortInfo struct {
	Port    uint8            `json:"port"`
	Devices []LogicalAddress `json:"devices"`
}

// BusTopology describes the HDMI bus as seen through CEC.
type BusTopology struct {
	OwnAddress     LogicalAddress `json:"own_address"`
	OwnPort        uint8          `json:"own_port"`          // HDMI port the adapter is on (0 = unknown)
	ActivePorts    []PortInfo     `json:"active_ports"`      // ports with at least one device
	KnownPortCount uint8          `json:"known_port_count"`  // highest port number observed
}

// GetBusTopology builds a topology of the CEC bus by inspecting the physical
// addresses of all active devices and grouping them by HDMI port.
func (c *Connection) GetBusTopology() *BusTopology {
	topo := &BusTopology{}

	// Determine the adapter's own address
	topo.OwnAddress = c.getOwnAddress()

	// Get adapter's physical address to determine which port it sits on
	if topo.OwnAddress != LogicalAddressFreeUse && topo.OwnAddress != LogicalAddressBroadcast {
		if physAddr, err := c.GetDevicePhysicalAddress(topo.OwnAddress); err == nil && physAddr != 0 && physAddr != 0xFFFF {
			topo.OwnPort = uint8((physAddr >> 12) & 0xF)
		}
	}

	// Collect all active devices and group by port
	portMap := make(map[uint8][]LogicalAddress)
	for _, addr := range c.GetActiveDevices() {
		// Skip TV (address 0) — it IS the display, not on a port
		if addr == LogicalAddressTV {
			continue
		}
		physAddr, err := c.GetDevicePhysicalAddress(addr)
		if err != nil || physAddr == 0 || physAddr == 0xFFFF {
			continue
		}
		port := uint8((physAddr >> 12) & 0xF)
		if port == 0 {
			continue // 0.x.x.x means internal / unknown
		}
		portMap[port] = append(portMap[port], addr)
		if port > topo.KnownPortCount {
			topo.KnownPortCount = port
		}
	}

	// Build sorted port list
	for p := uint8(1); p <= topo.KnownPortCount; p++ {
		if devs, ok := portMap[p]; ok {
			topo.ActivePorts = append(topo.ActivePorts, PortInfo{Port: p, Devices: devs})
		}
	}

	return topo
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

// GetVendorName returns a human-readable vendor name
func GetVendorName(vendorId uint64) string {
	vendors := map[uint64]string{
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

	if name, ok := vendors[vendorId]; ok {
		return name
	}
	return fmt.Sprintf("Unknown (0x%06X)", vendorId)
}

// PhysicalAddressToString converts a physical address to dot notation
func PhysicalAddressToString(addr uint16) string {
	a := (addr >> 12) & 0xF
	b := (addr >> 8) & 0xF
	c := (addr >> 4) & 0xF
	d := addr & 0xF
	return fmt.Sprintf("%d.%d.%d.%d", a, b, c, d)
}

// ParsePhysicalAddress converts dot notation to physical address
func ParsePhysicalAddress(addrStr string) (uint16, error) {
	var a, b, c, d uint16
	_, err := fmt.Sscanf(addrStr, "%d.%d.%d.%d", &a, &b, &c, &d)
	if err != nil {
		return 0, err
	}

	if a > 15 || b > 15 || c > 15 || d > 15 {
		return 0, fmt.Errorf("invalid physical address components (must be 0-15)")
	}

	return (a << 12) | (b << 8) | (c << 4) | d, nil
}

// SendButton sends a button press and release
func (c *Connection) SendButton(address LogicalAddress, key Keycode) error {
	// Send press
	if err := c.SendKeypress(address, key, false); err != nil {
		return err
	}

	// Wait a bit
	time.Sleep(100 * time.Millisecond)

	// Send release
	return c.SendKeyRelease(address, false)
}

// NavigateMenu sends navigation commands
func (c *Connection) NavigateMenu(address LogicalAddress, direction Keycode) error {
	return c.SendButton(address, direction)
}

// SetVolume sets absolute volume (if supported by device)
// This is a helper that sends multiple volume up/down commands
func (c *Connection) SetVolume(targetLevel int, currentLevel int) error {
	if targetLevel == currentLevel {
		return nil
	}

	if targetLevel > currentLevel {
		// Volume up
		steps := targetLevel - currentLevel
		for i := 0; i < steps; i++ {
			if err := c.VolumeUp(true); err != nil {
				return err
			}
			time.Sleep(100 * time.Millisecond)
		}
	} else {
		// Volume down
		steps := currentLevel - targetLevel
		for i := 0; i < steps; i++ {
			if err := c.VolumeDown(true); err != nil {
				return err
			}
			time.Sleep(100 * time.Millisecond)
		}
	}

	return nil
}

// MonitorConnection monitors the connection and reconnects if needed
func (c *Connection) MonitorConnection(reconnectFunc func() error) {
	// This can be called in a goroutine to monitor connection health
	// and attempt reconnection if the adapter becomes unavailable
	ticker := time.NewTicker(5 * time.Second)
	defer ticker.Stop()

	const maxConsecutiveFailures = 3
	failures := 0

	for range ticker.C {
		// Use a simple health check that returns an error on failure.
		if _, err := c.GetDevicePowerStatus(LogicalAddressTV); err != nil {
			failures++
		} else {
			failures = 0
		}

		if failures >= maxConsecutiveFailures && reconnectFunc != nil {
			_ = reconnectFunc()
			failures = 0
		}
	}
}
