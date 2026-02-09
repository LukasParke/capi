# API Reference

Complete HTTP API reference for CEC HTTP Bridge.

## Base URL

```
http://localhost:8080/api
```

## Response Format

All endpoints return JSON responses in the following format:

```json
{
  "status": "success|error",
  "message": "Human readable message",
  "data": {} // Optional, contains response data
}
```

## Error Codes

- `200` - Success
- `400` - Bad Request (invalid parameters)
- `500` - Internal Server Error (CEC operation failed)

---

## Device Endpoints

### GET /api/devices

List active CEC devices on the bus.

By default this uses cached device information to keep the call fast. To
force a full bus rescan before returning devices, pass `?rescan=1` (or
`?rescan=true`).

**Response:**
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

**Logical Addresses:**
- `0` - TV
- `1` - Recording Device 1
- `2` - Recording Device 2  
- `3` - Tuner 1
- `4` - Playback Device 1
- `5` - Audio System
- `6` - Tuner 2
- `7` - Tuner 3
- `8` - Playback Device 2
- `9` - Recording Device 3
- `10` - Tuner 4
- `11` - Playback Device 3
- `15` - Broadcast

### GET /api/devices/{address}

Get detailed information about a specific device.

**Parameters:**
- `address` (path) - Logical address (0-15)

**Example:**
```bash
curl http://localhost:8080/api/devices/0
```

**Response:**
```json
{
  "status": "success",
  "message": "Device info retrieved",
  "data": {
    "logical_address": 0,
    "address_name": "TV",
    "physical_address": "0.0.0.0",
    "vendor_id": "0x0000F0",
    "vendor_name": "Samsung",
    "cec_version": "1.4",
    "power_status": "On",
    "osd_name": "Samsung TV",
    "menu_language": "eng",
    "is_active": true,
    "is_active_source": true
  }
}
```

---

## Power Control Endpoints

### POST /api/power/on

Power on the TV (logical address 0).

**Example:**
```bash
curl -X POST http://localhost:8080/api/power/on
```

### POST /api/power/on/{address}

Power on a specific device.

**Parameters:**
- `address` (path) - Logical address (0-15)

**Example:**
```bash
curl -X POST http://localhost:8080/api/power/on/4
```

### POST /api/power/off

Put TV in standby mode.

**Example:**
```bash
curl -X POST http://localhost:8080/api/power/off
```

### POST /api/power/off/{address}

Put a specific device in standby.

**Parameters:**
- `address` (path) - Logical address (0-15)

**Example:**
```bash
curl -X POST http://localhost:8080/api/power/off/4
```

### GET /api/power/status

Get power status of TV.

**Response:**
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

**Power Status Values:**
- `On` - Device is on
- `Standby` - Device is in standby
- `Transitioning to On` - Device is powering on
- `Transitioning to Standby` - Device is powering off
- `Unknown` - Status could not be determined

### GET /api/power/status/{address}

Get power status of specific device.

**Parameters:**
- `address` (path) - Logical address (0-15)

---

## Volume Control Endpoints

### POST /api/volume/up

Increase system volume by one step.

**Example:**
```bash
curl -X POST http://localhost:8080/api/volume/up
```

### POST /api/volume/down

Decrease system volume by one step.

**Example:**
```bash
curl -X POST http://localhost:8080/api/volume/down
```

### POST /api/volume/mute

Toggle audio mute.

**Example:**
```bash
curl -X POST http://localhost:8080/api/volume/mute
```

**Note:** Volume commands are sent to the audio system (usually TV or receiver).

---

## Source Control Endpoints

### GET /api/source/active

Get the currently active source.

**Response:**
```json
{
  "status": "success",
  "message": "Active source retrieved",
  "data": {
    "address": 4,
    "name": "Playback Device 1"
  }
}
```

### POST /api/source/{address}

Switch to a specific device by logical address.

This command:
1. Powers on the device if it's in standby
2. Sends "Set Stream Path" to switch to the device

**Parameters:**
- `address` (path) - Logical address (0-15)

**Example:**
```bash
curl -X POST http://localhost:8080/api/source/4
```

### POST /api/hdmi/{port}

Switch TV input to a specific HDMI port.

**Parameters:**
- `port` (path) - HDMI port number (1-4)

**Example:**
```bash
# Switch to HDMI 2
curl -X POST http://localhost:8080/api/hdmi/2
```

**Physical Address Mapping:**
- Port 1: `1.0.0.0` (0x1000)
- Port 2: `2.0.0.0` (0x2000)
- Port 3: `3.0.0.0` (0x3000)
- Port 4: `4.0.0.0` (0x4000)

---

## Navigation Endpoints

### POST /api/key

Send a key press to a device (press and release).

**Request Body:**
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

**Parameters:**
- `address` (integer, required) - Target device logical address (0-15)
- `key` (string, optional) - Key name. If provided, must be one of the
  supported names below.
- `keycode` (integer, optional) - Raw keycode (0-255). If you need to send
  keycode `0` (Select), you **must** specify it via `"key": "select"` rather
  than relying on the default value.

Exactly one of `key` or `keycode` must be valid. Requests that omit both,
or use an unsupported key name, are rejected with `400 Bad Request`.

**Supported Key Names:**
- `up`, `down`, `left`, `right` - Navigation
- `select`, `enter` - Select/confirm
- `back`, `exit` - Go back
- `home`, `menu` - Home/menu buttons
- `play`, `pause`, `stop` - Playback control
- `rewind`, `forward` - Seek control

**Common Keycodes:**
- `0` - Select
- `1` - Up
- `2` - Down
- `3` - Left
- `4` - Right
- `9` - Root Menu
- `13` - Exit
- `32-41` - Numbers 0-9
- `64` - Power
- `65` - Volume Up
- `66` - Volume Down
- `67` - Mute
- `68` - Play
- `69` - Stop
- `70` - Pause

**Example:**
```bash
# Send up key to playback device
curl -X POST http://localhost:8080/api/key \
  -H "Content-Type: application/json" \
  -d '{"address": 4, "key": "up"}'

# Send select using keycode
curl -X POST http://localhost:8080/api/key \
  -H "Content-Type: application/json" \
  -d '{"address": 4, "keycode": 0}'
```

---

## Raw Command Endpoint

### POST /api/command

Send a raw CEC command.

**Request Body:**
```json
{
  "initiator": 1,
  "destination": 0,
  "opcode": 143,
  "parameters": [0, 1]
}
```

**Parameters:**
- `initiator` (integer, required) - Source logical address (0-15)
- `destination` (integer, required) - Target logical address (0-15)
- `opcode` (integer, required) - CEC opcode (0-255)
- `parameters` (array, optional) - Parameter bytes. At most 14 bytes are
  accepted for a single frame; longer payloads are rejected with
  `400 Bad Request`.

**Common Opcodes:**
- `0x82` (130) - Active Source
- `0x04` (4) - Image View On
- `0x0D` (13) - Text View On
- `0x36` (54) - Standby
- `0x44` (68) - User Control Pressed
- `0x45` (69) - User Control Released
- `0x8F` (143) - Give Device Power Status
- `0x90` (144) - Report Power Status
- `0x85` (133) - Request Active Source
- `0x86` (134) - Set Stream Path
- `0x8C` (140) - Give Device Vendor ID
- `0x87` (135) - Device Vendor ID

**Example - Request Power Status:**
```bash
curl -X POST http://localhost:8080/api/command \
  -H "Content-Type: application/json" \
  -d '{
    "initiator": 1,
    "destination": 0,
    "opcode": 143,
    "parameters": []
  }'
```

**Example - Set Stream Path to HDMI 2:**
```bash
curl -X POST http://localhost:8080/api/command \
  -H "Content-Type: application/json" \
  -d '{
    "initiator": 1,
    "destination": 15,
    "opcode": 134,
    "parameters": [32, 0]
  }'
```

---

## System Endpoints

### GET /api/health

Health check and version information.

**Response:**
```json
{
  "status": "success",
  "message": "Service is healthy",
  "data": {
    "version": "1.0.0",
    "libcec": "CEC Parser created - libCEC version 6.0.2"
  }
}
```

### GET /api/logs

Get recent CEC log messages.

**Response:**
```json
{
  "status": "success",
  "message": "Logs retrieved",
  "data": [
    {
      "level": "NOTICE",
      "timestamp": "2024-02-08T10:30:45Z",
      "message": "CEC connection opened"
    },
    {
      "level": "TRAFFIC",
      "timestamp": "2024-02-08T10:30:46Z",
      "message": ">> 10:8f"
    }
  ]
}
```

**Log Levels:**
- `ERROR` - Error messages
- `WARNING` - Warning messages
- `NOTICE` - Important notices
- `TRAFFIC` - CEC bus traffic
- `DEBUG` - Debug messages

---

## Complete Examples

### Morning Routine

```bash
#!/bin/bash
API="http://localhost:8080/api"

# Power on TV
curl -X POST "$API/power/on"
sleep 2

# Switch to HDMI 1 (cable box)
curl -X POST "$API/hdmi/1"
sleep 1

# Unmute and set volume
curl -X POST "$API/volume/mute"
for i in {1..10}; do
    curl -X POST "$API/volume/up"
    sleep 0.2
done
```

### Device Discovery

```bash
#!/bin/bash
API="http://localhost:8080/api"

# Get all devices
devices=$(curl -s "$API/devices" | jq -r '.data[] | "\(.logical_address): \(.address_name) - \(.osd_name) (\(.vendor_name))"')

echo "CEC Devices:"
echo "$devices"

# Get power status for each
curl -s "$API/devices" | jq -r '.data[].logical_address' | while read addr; do
    status=$(curl -s "$API/power/status/$addr" | jq -r '.data.status')
    echo "Device $addr: $status"
done
```

### HDMI Switcher

```bash
#!/bin/bash
API="http://localhost:8080/api"

echo "Select HDMI Input:"
echo "1) HDMI 1"
echo "2) HDMI 2"
echo "3) HDMI 3"
echo "4) HDMI 4"
read -p "Choice: " choice

curl -X POST "$API/hdmi/$choice"
echo "Switched to HDMI $choice"
```

### Volume Control

```python
import requests
import sys

API = "http://localhost:8080/api"

def set_volume(level):
    """Set volume to specific level (0-100)"""
    # This is approximate - actual steps depend on device
    target_steps = level // 2
    
    # Mute then gradually increase
    requests.post(f"{API}/volume/mute")
    
    for i in range(target_steps):
        requests.post(f"{API}/volume/up")
        time.sleep(0.1)

if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: volume.py <0-100>")
        sys.exit(1)
    
    level = int(sys.argv[1])
    set_volume(level)
```

---

## Integration Examples

### Home Assistant

```yaml
# configuration.yaml
rest_command:
  cec_power_on:
    url: "http://192.168.1.100:8080/api/power/on"
    method: POST
  
  cec_power_off:
    url: "http://192.168.1.100:8080/api/power/off"
    method: POST
  
  cec_hdmi:
    url: "http://192.168.1.100:8080/api/hdmi/{{ port }}"
    method: POST

automation:
  - alias: "Morning TV"
    trigger:
      platform: time
      at: "07:00:00"
    action:
      - service: rest_command.cec_power_on
      - delay: "00:00:02"
      - service: rest_command.cec_hdmi
        data:
          port: 1
```

### Node-RED

```json
[
    {
        "id": "cec_power",
        "type": "http request",
        "method": "POST",
        "url": "http://localhost:8080/api/power/on",
        "name": "CEC Power On"
    }
]
```

### Python Script

```python
import requests
from typing import List, Dict

class CECClient:
    def __init__(self, base_url: str = "http://localhost:8080/api"):
        self.base_url = base_url
    
    def get_devices(self) -> List[Dict]:
        response = requests.get(f"{self.base_url}/devices")
        return response.json()['data']
    
    def power_on(self, address: int = 0):
        requests.post(f"{self.base_url}/power/on/{address}")
    
    def power_off(self, address: int = 0):
        requests.post(f"{self.base_url}/power/off/{address}")
    
    def switch_hdmi(self, port: int):
        requests.post(f"{self.base_url}/hdmi/{port}")
    
    def send_key(self, address: int, key: str):
        requests.post(f"{self.base_url}/key", 
                     json={"address": address, "key": key})

# Usage
cec = CECClient()
cec.power_on()
cec.switch_hdmi(2)
```
