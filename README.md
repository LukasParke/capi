# capi

A comprehensive Go package and HTTP service for controlling HDMI-CEC devices. Provides full libcec bindings with idiomatic Go interfaces and a REST API for remote control.

**Deployment:** Build and run directly on a Raspberry Pi (or similar); no cross-compilation. Use `make setup` then `make deploy` on the device.

## Features

- **Complete libcec bindings** via cgo
- **Full CEC support**: power control, volume, input switching, device queries
- **Reading capabilities**: device status, power state, active source, vendor info
- **HTTP REST API** for remote access
- **Callbacks** for CEC events
- **Helper functions** for common operations
- **Systemd service** integration

## Architecture

### Components

1. **cec** package - Complete Go wrapper around libcec
   - `libcec.go` - Main API wrapper
   - `types.go` - CEC types and constants
   - `callbacks.go` - Event callback system
   - `helpers.go` - Convenience functions

2. **capi** - HTTP service
   - RESTful API exposing all CEC functionality
   - JSON request/response format
   - Event logging

## Installation

Target: **Raspberry Pi** (or other ARM SBC). Build natively on the device.

### Prerequisites

On the Pi (Raspberry Pi OS / Debian / Ubuntu):

```bash
# Install libcec and build tools
sudo apt-get update
sudo apt-get install -y libcec-dev libcec6 cec-utils pkg-config

# Install Go 1.25+ (Raspberry Pi OS: sudo apt-get install -y golang-go, or from https://go.dev/dl/ — linux/arm64 or armv6l)
```

Or run `sudo make setup` to install the above (except Go, which you may install separately).

### Building

On the Pi:

```bash
cd /path/to/capi
go mod tidy
make build
```

**Note:** The `platform/` directory (Pulse-Eight p8-platform) is in `.gitignore`. It is not required when building with the system `libcec-dev` package. Only clone or add it if you are building libcec from source.

### Installation as System Service

```bash
# One-command deploy (install, enable, and start)
sudo make deploy

# Or step by step:
sudo make install
sudo systemctl enable capi
sudo systemctl start capi

# Check status
sudo systemctl status capi

# View logs
sudo journalctl -u capi -f
```

The service runs as the system user `capi`. Udev rules in `99-cec.rules` grant the `capi` group access to Pulse-Eight CEC adapters.

## Usage

### Command Line

```bash
# Run with default settings (bind to all interfaces on port 8080)
./capi

# Bind to localhost only
./capi -bind localhost:8080

# Bind to all interfaces on custom port
./capi -bind :9090

# Specify device name
./capi -name "My Media Center"

# Specify adapter path
./capi -adapter /dev/ttyACM0
```

### Go Package Usage

```go
package main

import (
    "fmt"
    "log"
    "capi/cec"
)

func main() {
    // Open CEC connection
    conn, err := cec.Open("My Device", cec.DeviceTypePlaybackDevice)
    if err != nil {
        log.Fatal(err)
    }
    defer conn.Close()
    
    // Find and open adapter
    adapters, _ := conn.FindAdapters()
    conn.OpenAdapter(adapters[0].Path)
    
    // Power on TV
    conn.PowerOn(cec.LogicalAddressTV)
    
    // Get all devices
    devices, _ := conn.GetAllDevices()
    for _, dev := range devices {
        fmt.Printf("Device: %s (%s)\n", dev.OSDName, dev.LogicalAddress.String())
        fmt.Printf("  Power: %s\n", dev.PowerStatus.String())
        fmt.Printf("  Vendor: %s\n", cec.GetVendorName(dev.VendorID))
    }
    
    // Switch to HDMI 2
    conn.SwitchToHDMIPort(2)
    
    // Volume control
    conn.VolumeUp(true)
    
    // Send navigation keys
    conn.SendButton(cec.LogicalAddressPlaybackDevice1, cec.KeycodeSelect)
    
    // Raw command
    cmd := &cec.Command{
        Initiator:   cec.LogicalAddressPlaybackDevice1,
        Destination: cec.LogicalAddressTV,
        Opcode:      cec.OpcodeGiveDevicePowerStatus,
        OpcodeSet:   true,
    }
    conn.Transmit(cmd)
}
```

### Custom Callbacks

```go
type MyHandler struct {
    cec.DefaultCallbackHandler
}

func (h *MyHandler) OnKeyPress(key cec.Keycode, duration uint32) {
    fmt.Printf("Key pressed: %d\n", key)
}

func (h *MyHandler) OnCommand(command *cec.Command) {
    fmt.Printf("Command: %s -> %s, opcode: 0x%02X\n",
        command.Initiator.String(), 
        command.Destination.String(), 
        command.Opcode)
}

// Use it
conn.SetCallbackHandler(&MyHandler{})
```

## HTTP API

### Base URL

```
http://localhost:8080/api
```

### Endpoints

#### Device Information

**GET /api/devices**
List all active CEC devices

Response:
```json
{
  "status": "success",
  "message": "Devices retrieved",
  "data": [
    {
      "logical_address": 0,
      "address_name": "TV",
      "physical_address": "0.0.0.0",
      "vendor_id": "0x0000F0",
      "vendor_name": "Samsung",
      "cec_version": "1.4",
      "power_status": "On",
      "osd_name": "TV",
      "menu_language": "eng",
      "is_active": true,
      "is_active_source": false
    }
  ]
}
```

**GET /api/devices/{address}**
Get info for specific device (address: 0-15)

#### Power Control

**POST /api/power/on**
**POST /api/power/on/{address}**
Power on device (default: TV)

**POST /api/power/off**
**POST /api/power/off/{address}**
Put device in standby

**GET /api/power/status**
**GET /api/power/status/{address}**
Get power status

Response:
```json
{
  "status": "success",
  "message": "Power status retrieved",
  "data": {
    "address": 0,
    "status": "On"
  }
}
```

#### Volume Control

**POST /api/volume/up**
Increase volume

**POST /api/volume/down**
Decrease volume

**POST /api/volume/mute**
Toggle mute

#### Source Control

**GET /api/source/active**
Get current active source

**POST /api/source/{address}**
Switch to device by logical address

**POST /api/hdmi/{port}**
Switch TV to HDMI port (1-4)

Example:
```bash
curl -X POST http://localhost:8080/api/hdmi/2
```

#### Navigation

**POST /api/key**
Send key press

Request:
```json
{
  "address": 4,
  "key": "select"
}
```

Or with keycode:
```json
{
  "address": 4,
  "keycode": 0
}
```

Supported key names:
- `up`, `down`, `left`, `right`
- `select`, `enter`, `back`
- `home`, `menu`
- `play`, `pause`, `stop`

#### Raw Commands

**POST /api/command**
Send raw CEC command

Request:
```json
{
  "initiator": 1,
  "destination": 0,
  "opcode": 143,
  "parameters": [0, 1]
}
```

#### System

**GET /api/health**
Health check and version info

**GET /api/logs**
Get recent CEC log messages

## API Examples

### Using curl

```bash
# Get all devices
curl http://localhost:8080/api/devices

# Power on TV
curl -X POST http://localhost:8080/api/power/on

# Switch to HDMI 2
curl -X POST http://localhost:8080/api/hdmi/2

# Volume up
curl -X POST http://localhost:8080/api/volume/up

# Send key press
curl -X POST http://localhost:8080/api/key \
  -H "Content-Type: application/json" \
  -d '{"address": 4, "key": "select"}'

# Get power status
curl http://localhost:8080/api/power/status

# Raw command (get device vendor ID)
curl -X POST http://localhost:8080/api/command \
  -H "Content-Type: application/json" \
  -d '{"initiator": 1, "destination": 0, "opcode": 140, "parameters": []}'
```

### Using Python

```python
import requests

BASE_URL = "http://localhost:8080/api"

# Get devices
response = requests.get(f"{BASE_URL}/devices")
devices = response.json()['data']

# Power on TV
requests.post(f"{BASE_URL}/power/on")

# Switch to HDMI 2
requests.post(f"{BASE_URL}/hdmi/2")

# Send navigation
requests.post(f"{BASE_URL}/key", json={
    "address": 4,
    "key": "up"
})
```

### Using JavaScript/Node.js

```javascript
const axios = require('axios');

const BASE_URL = 'http://localhost:8080/api';

async function main() {
  // Get devices
  const devices = await axios.get(`${BASE_URL}/devices`);
  console.log(devices.data);
  
  // Power on TV
  await axios.post(`${BASE_URL}/power/on`);
  
  // Switch to HDMI 2
  await axios.post(`${BASE_URL}/hdmi/2`);
  
  // Volume control
  await axios.post(`${BASE_URL}/volume/up`);
}

main();
```

## CEC Package API Reference

### Connection Methods

- `Open(deviceName, deviceType)` - Create new connection
- `OpenWithConfig(config)` - Create with custom config
- `Close()` - Close connection
- `FindAdapters()` - List available adapters
- `OpenAdapter(path)` - Open specific adapter

### Power Control

- `PowerOn(address)` - Power on device
- `Standby(address)` - Put in standby
- `GetDevicePowerStatus(address)` - Get power state

### Source Control

- `SetActiveSource(deviceType)` - Set as active source
- `GetActiveSource()` - Get active source address
- `IsActiveSource(address)` - Check if device is active
- `SwitchToDevice(address)` - Switch to device
- `SwitchToHDMIPort(port)` - Switch to HDMI port

### Volume Control

- `VolumeUp(sendRelease)` - Increase volume
- `VolumeDown(sendRelease)` - Decrease volume
- `AudioToggleMute()` - Toggle mute
- `AudioMute()` - Mute
- `AudioUnmute()` - Unmute

### Device Information

- `GetDeviceInfo(address)` - Get all device info
- `GetAllDevices()` - Scan and get all devices
- `GetDeviceVendorId(address)` - Get vendor ID
- `GetDevicePhysicalAddress(address)` - Get physical address
- `GetDeviceOSDName(address)` - Get OSD name
- `GetDeviceMenuLanguage(address)` - Get menu language
- `GetDeviceCecVersion(address)` - Get CEC version
- `GetActiveDevices()` - Get list of active addresses
- `IsActiveDevice(address)` - Check if active

### Navigation & Keys

- `SendKeypress(address, key, wait)` - Send key press
- `SendKeyRelease(address, wait)` - Send key release
- `SendButton(address, key)` - Press and release

### Raw Commands

- `Transmit(command)` - Send raw CEC command

### Configuration

- `SetConfiguration(config)` - Update config
- `GetCurrentConfiguration()` - Get current config
- `RescanDevices()` - Rescan CEC bus

### Utilities

- `GetVendorName(vendorId)` - Vendor ID to name
- `PhysicalAddressToString(addr)` - Format physical address
- `ParsePhysicalAddress(str)` - Parse dot notation

## Systemd Service

Create `/etc/systemd/system/capi.service`:

```ini
[Unit]
Description=CEC HTTP Bridge Service
After=network.target

[Service]
Type=simple
User=pi
WorkingDirectory=/opt/capi
ExecStart=/opt/capi/capi -bind :8080
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

## Configuration Options

### Binding

- `:8080` - All interfaces, port 8080
- `localhost:8080` - Local only
- `192.168.1.100:8080` - Specific IP

### Device Types

- `DeviceTypeTV`
- `DeviceTypeRecordingDevice`
- `DeviceTypePlaybackDevice`
- `DeviceTypeAudioSystem`
- `DeviceTypeTuner`

## Troubleshooting

### No adapters found

```bash
# Check if adapter is connected
ls -la /dev/ttyACM*

# Check cec-client
cec-client -l

# Check permissions
sudo usermod -a -G dialout $USER
# Log out and back in
```

### Connection fails

```bash
# Test with cec-client
echo "scan" | cec-client -s -d 8

# Check libcec version
dpkg -l | grep libcec
```

### Service won't start

```bash
# Check logs
sudo journalctl -u capi -n 50

# Test manually
sudo -u pi /opt/capi/capi

# Check permissions
ls -la /dev/ttyACM0
```

## Development

### Building for Debug

```bash
# Build with debug info
go build -gcflags="all=-N -l" -o capi ./capi

# Run with verbose libcec logging
./capi
```

### Testing

```bash
# Test CEC package
go test ./cec -v

# Test with actual hardware
go run ./examples/test-connection.go
```

## Hardware Compatibility

Tested with:
- Raspberry Pi (built-in CEC)
- USB-CEC adapters (Pulse-Eight)
- Most HDMI-CEC capable devices

## License

MIT License

## Contributing

Contributions welcome! Please ensure:
- Code compiles without warnings
- Follows Go conventions
- Includes documentation
- Tests pass

## Credits

Built on [libcec](https://github.com/Pulse-Eight/libcec) by Pulse-Eight.
