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

// SwitchToHDMIPort switches TV input to a specific HDMI port
func (c *Connection) SwitchToHDMIPort(port uint8) error {
	// Standard physical address mapping for HDMI ports
	// Port 1: 1.0.0.0 (0x1000)
	// Port 2: 2.0.0.0 (0x2000)
	// Port 3: 3.0.0.0 (0x3000)
	// Port 4: 4.0.0.0 (0x4000)

	if port < 1 || port > 4 {
		return fmt.Errorf("invalid HDMI port %d (must be 1-4)", port)
	}

	physicalAddress := uint16(port) << 12 // Shift port number to first nibble

	// Send Set Stream Path command
	cmd := &Command{
		Initiator:   LogicalAddressPlaybackDevice1,
		Destination: LogicalAddressBroadcast,
		Opcode:      OpcodeSetStreamPath,
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
	// First power on the device if needed
	if err := c.PowerOn(address); err != nil {
		return fmt.Errorf("failed to power on device: %w", err)
	}

	// Wait a bit for device to power on
	time.Sleep(500 * time.Millisecond)

	// Get device's physical address
	physAddr, err := c.GetDevicePhysicalAddress(address)
	if err != nil {
		return fmt.Errorf("failed to get physical address: %w", err)
	}

	// Send Set Stream Path to the device's physical address
	cmd := &Command{
		Initiator:   LogicalAddressPlaybackDevice1,
		Destination: LogicalAddressBroadcast,
		Opcode:      OpcodeSetStreamPath,
		OpcodeSet:   true,
		Parameters: []uint8{
			uint8(physAddr >> 8),
			uint8(physAddr & 0xFF),
		},
	}

	return c.Transmit(cmd)
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
