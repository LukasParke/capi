package main

import (
	"fmt"
	"log"
	"time"

	"capi/cec"
)

// Example callback handler
type ExampleHandler struct {
	cec.DefaultCallbackHandler
}

func (h *ExampleHandler) OnLogMessage(level cec.LogLevel, timestamp int64, message string) {
	if level == cec.LogLevelError || level == cec.LogLevelWarning {
		log.Printf("[%s] %s", level.String(), message)
	}
}

func (h *ExampleHandler) OnKeyPress(key cec.Keycode, duration uint32) {
	fmt.Printf("Key pressed: %d, duration: %d ms\n", key, duration)
}

func (h *ExampleHandler) OnCommand(command *cec.Command) {
	fmt.Printf("Command: %s -> %s, opcode: 0x%02X\n",
		command.Initiator.String(),
		command.Destination.String(),
		command.Opcode)
}

func main() {
	fmt.Println("CEC Library Example")
	fmt.Println("===================")

	// Initialize CEC connection
	fmt.Println("\n1. Initializing CEC connection...")
	conn, err := cec.Open("Example Device", cec.DeviceTypePlaybackDevice)
	if err != nil {
		log.Fatalf("Failed to initialize CEC: %v", err)
	}
	defer conn.Close()

	// Set callback handler
	handler := &ExampleHandler{}
	conn.SetCallbackHandler(handler)

	// Find adapters
	fmt.Println("\n2. Searching for CEC adapters...")
	adapters, err := conn.FindAdapters()
	if err != nil {
		log.Fatalf("Failed to find adapters: %v", err)
	}

	if len(adapters) == 0 {
		log.Fatal("No CEC adapters found")
	}

	fmt.Printf("Found %d adapter(s):\n", len(adapters))
	for i, adapter := range adapters {
		fmt.Printf("  [%d] %s (%s)\n", i, adapter.Path, adapter.Comm)
	}

	// Open the first adapter
	fmt.Printf("\n3. Opening adapter: %s\n", adapters[0].Path)
	if err := conn.OpenAdapter(adapters[0].Path); err != nil {
		log.Fatalf("Failed to open adapter: %v", err)
	}

	// Display library info
	fmt.Printf("\n4. Library info:\n%s\n", conn.GetLibInfo())

	// Wait for CEC to initialize
	time.Sleep(2 * time.Second)

	// Scan for devices
	fmt.Println("\n5. Scanning for devices...")
	if err := conn.RescanDevices(); err != nil {
		log.Printf("Warning: Rescan failed: %v", err)
	}

	devices, err := conn.GetAllDevices()
	if err != nil {
		log.Printf("Warning: Failed to get devices: %v", err)
	} else {
		fmt.Printf("Found %d device(s):\n", len(devices))
		for _, dev := range devices {
			fmt.Printf("\n  Device: %s\n", dev.LogicalAddress.String())
			fmt.Printf("    Logical Address:  %d\n", dev.LogicalAddress)
			fmt.Printf("    Physical Address: %s\n", cec.PhysicalAddressToString(dev.PhysicalAddress))
			fmt.Printf("    OSD Name:         %s\n", dev.OSDName)
			fmt.Printf("    Vendor:           %s\n", cec.GetVendorName(dev.VendorID))
			fmt.Printf("    CEC Version:      %s\n", dev.CECVersion.String())
			fmt.Printf("    Power Status:     %s\n", dev.PowerStatus.String())
			fmt.Printf("    Active:           %v\n", dev.IsActive)
			fmt.Printf("    Active Source:    %v\n", dev.IsActiveSource)
		}
	}

	// Get active source
	fmt.Println("\n6. Getting active source...")
	activeSource, err := conn.GetActiveSource()
	if err != nil {
		log.Printf("Failed to get active source: %v", err)
	} else {
		fmt.Printf("Active source: %s (%d)\n", activeSource.String(), activeSource)
	}

	// Example operations
	fmt.Println("\n7. Demonstrating CEC operations...")

	// Power on TV
	fmt.Println("  - Powering on TV...")
	if err := conn.PowerOn(cec.LogicalAddressTV); err != nil {
		log.Printf("Failed to power on TV: %v", err)
	}
	time.Sleep(1 * time.Second)

	// Get TV power status
	fmt.Println("  - Checking TV power status...")
	status, err := conn.GetDevicePowerStatus(cec.LogicalAddressTV)
	if err != nil {
		log.Printf("Failed to get power status: %v", err)
	} else {
		fmt.Printf("    TV power status: %s\n", status.String())
	}

	// Volume control example
	fmt.Println("  - Volume up...")
	if err := conn.VolumeUp(true); err != nil {
		log.Printf("Failed to increase volume: %v", err)
	}
	time.Sleep(500 * time.Millisecond)

	// HDMI port switching example
	fmt.Println("  - Switching to HDMI port 2...")
	if err := conn.SwitchToHDMIPort(2); err != nil {
		log.Printf("Failed to switch HDMI port: %v", err)
	}
	time.Sleep(1 * time.Second)

	// Navigation example
	fmt.Println("  - Sending navigation key (Up)...")
	if err := conn.SendButton(cec.LogicalAddressPlaybackDevice1, cec.KeycodeUp); err != nil {
		log.Printf("Failed to send button: %v", err)
	}

	// Raw command example
	fmt.Println("  - Sending raw command (Request Active Source)...")
	cmd := &cec.Command{
		Initiator:   cec.LogicalAddressPlaybackDevice1,
		Destination: cec.LogicalAddressBroadcast,
		Opcode:      cec.OpcodeRequestActiveSource,
		OpcodeSet:   true,
	}
	if err := conn.Transmit(cmd); err != nil {
		log.Printf("Failed to transmit command: %v", err)
	}

	// Wait to observe any callbacks
	fmt.Println("\n8. Waiting for callbacks (5 seconds)...")
	time.Sleep(5 * time.Second)

	fmt.Println("\n9. Example complete!")
}
