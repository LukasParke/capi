package main

import (
	"fmt"
	"log"
	"time"

	"github.com/LukasParke/capi/cec"
)

func main() {
	fmt.Println("CEC Library Example")
	fmt.Println("===================")

	fmt.Println("\n1. Initializing CEC connection...")
	conn, err := cec.Open("Example Device", cec.DeviceTypePlaybackDevice)
	if err != nil {
		log.Fatalf("Failed to initialize CEC: %v", err)
	}
	defer conn.Close()

	// Drain events on a goroutine; the channel is closed by Close.
	go func() {
		for ev := range conn.Events() {
			switch ev.Kind {
			case cec.EventLog:
				if ev.Log != nil && (ev.Log.Level == cec.LogLevelError || ev.Log.Level == cec.LogLevelWarning) {
					log.Printf("[%s] %s", ev.Log.Level, ev.Log.Message)
				}
			case cec.EventKeyPress:
				if ev.Key != nil {
					fmt.Printf("Key pressed: %d, duration: %d ms\n", ev.Key.Key, ev.Key.Duration)
				}
			case cec.EventCommand:
				if ev.Command != nil {
					fmt.Printf("Command: %s -> %s, opcode: 0x%02X\n",
						ev.Command.Initiator, ev.Command.Destination, ev.Command.Opcode)
				}
			}
		}
	}()

	fmt.Println("\n2. Searching for CEC adapters...")
	adapters, err := conn.FindAdapters()
	if err != nil {
		log.Fatalf("Failed to find adapters: %v", err)
	}
	if len(adapters) == 0 {
		log.Fatal("No CEC adapters found")
	}
	fmt.Printf("Found %d adapter(s):\n", len(adapters))
	for i, a := range adapters {
		fmt.Printf("  [%d] %s (%s)\n", i, a.Path, a.Comm)
	}

	fmt.Printf("\n3. Opening adapter: %s\n", adapters[0].Path)
	if err := conn.OpenAdapter(adapters[0].Path); err != nil {
		log.Fatalf("Failed to open adapter: %v", err)
	}

	fmt.Printf("\n4. Library info:\n%s\n", conn.GetLibInfo())

	time.Sleep(2 * time.Second)

	fmt.Println("\n5. Scanning for devices...")
	devices, err := conn.GetAllDevices(2 * time.Second)
	if err != nil {
		log.Printf("Warning: rescan failed: %v", err)
	} else {
		fmt.Printf("Found %d device(s):\n", len(devices))
		for _, dev := range devices {
			fmt.Printf("\n  Device: %s\n", dev.LogicalAddress)
			fmt.Printf("    Logical Address:  %d\n", dev.LogicalAddress)
			fmt.Printf("    Physical Address: %s\n", cec.PhysicalAddressToString(dev.PhysicalAddress))
			fmt.Printf("    OSD Name:         %s\n", dev.OSDName)
			fmt.Printf("    Vendor:           %s\n", cec.GetVendorName(dev.VendorID))
			fmt.Printf("    CEC Version:      %s\n", dev.CECVersion)
			fmt.Printf("    Power Status:     %s\n", dev.PowerStatus)
			fmt.Printf("    Active:           %v\n", dev.IsActive)
			fmt.Printf("    Active Source:    %v\n", dev.IsActiveSource)
		}
	}

	fmt.Println("\n6. Getting active source...")
	if active, err := conn.GetActiveSource(); err != nil {
		log.Printf("active source: %v", err)
	} else {
		fmt.Printf("Active source: %s (%d)\n", active, active)
	}

	fmt.Println("\n7. Demonstrating CEC operations...")
	fmt.Println("  - Powering on TV...")
	if err := conn.PowerOn(cec.LogicalAddressTV); err != nil {
		log.Printf("power on tv: %v", err)
	}
	time.Sleep(1 * time.Second)

	fmt.Println("  - Checking TV power status...")
	if status, err := conn.GetDevicePowerStatus(cec.LogicalAddressTV); err != nil {
		log.Printf("power status: %v", err)
	} else {
		fmt.Printf("    TV power status: %s\n", status)
	}

	fmt.Println("  - Volume up...")
	if err := conn.VolumeUp(true); err != nil {
		log.Printf("volume up: %v", err)
	}
	time.Sleep(500 * time.Millisecond)

	fmt.Println("  - Switching to HDMI port 2...")
	if err := conn.SwitchToHDMIPort(2); err != nil {
		log.Printf("hdmi 2: %v", err)
	}
	time.Sleep(1 * time.Second)

	fmt.Println("  - Sending navigation key (Up)...")
	if err := conn.SendButton(cec.LogicalAddressPlaybackDevice1, cec.KeycodeUp); err != nil {
		log.Printf("send button: %v", err)
	}

	fmt.Println("  - Sending raw command (Request Active Source)...")
	if err := conn.Transmit(&cec.Command{
		Initiator:   cec.LogicalAddressPlaybackDevice1,
		Destination: cec.LogicalAddressBroadcast,
		Opcode:      cec.OpcodeRequestActiveSource,
		OpcodeSet:   true,
	}); err != nil {
		log.Printf("transmit: %v", err)
	}

	fmt.Println("\n8. Waiting for callbacks (5 seconds)...")
	time.Sleep(5 * time.Second)

	fmt.Println("\n9. Example complete!")
}
