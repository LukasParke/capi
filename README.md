# capi

HDMI-CEC control over HTTP and MQTT. A Go service with full [libcec](https://github.com/Pulse-Eight/libcec) bindings, a REST API, real-time Server-Sent Events, MQTT integration, a web dashboard, and over-the-air self-update. Designed for Raspberry Pi and ARM SBCs.

## Quick Install

Run this on the target device (Raspberry Pi, etc.) as root:

```bash
curl -sSL https://raw.githubusercontent.com/LukasParke/capi/main/install.sh | sudo bash
```

This downloads the latest release binary, installs systemd/udev files, and starts the service. Once running, open `http://<device-ip>:8080` in a browser.

To **update** an existing installation, run the same command again -- or use the web UI's one-click update button, or run `sudo /opt/capi/capi -update`.

## Features

- **Complete libcec bindings** via cgo with idiomatic Go interfaces
- **REST API** for power, volume, source/HDMI switching, navigation keys, raw CEC commands
- **MQTT bridge** -- publish CEC events and subscribe to command topics (opt-in)
- **Web dashboard** with live device status, remote control, and one-click update
- **Server-Sent Events** for real-time CEC bus monitoring
- **Self-update** from GitHub releases (CLI and web UI)
- **Systemd service** with security hardening and udev rules
- **Automatic adapter detection** (built-in HDMI CEC and USB Pulse-Eight adapters)

## Configuration

### CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `-bind` | `:8080` | Bind address (`:8080` for all interfaces, `localhost:8080` for local only) |
| `-name` | `CEC HTTP Bridge` | CEC device name on the bus |
| `-adapter` | (auto-detect) | CEC adapter path (e.g. `/dev/cec0`, `/dev/ttyACM0`) |
| `-mqtt-broker` | (disabled) | MQTT broker URL (e.g. `tcp://localhost:1883`). Empty disables MQTT. |
| `-mqtt-user` | | MQTT username |
| `-mqtt-pass` | | MQTT password |
| `-mqtt-prefix` | `capi` | MQTT topic prefix |
| `-version` | | Print version and exit |
| `-update` | | Check for updates and install the latest release |

### Examples

```bash
# Run with defaults (all interfaces, port 8080)
./capi

# Local only, custom port
./capi -bind localhost:9090

# With MQTT
./capi -mqtt-broker tcp://192.168.1.10:1883 -mqtt-user ha -mqtt-pass secret

# Specify adapter
./capi -adapter /dev/ttyACM0
```

### Systemd Service

The install script sets up `/etc/systemd/system/capi.service` running as the `capi` system user. To configure flags (e.g. enable MQTT), edit the service file:

```bash
sudo systemctl edit capi.service
```

Add an override for `ExecStart`:

```ini
[Service]
ExecStart=
ExecStart=/opt/capi/capi -bind :8080 -mqtt-broker tcp://localhost:1883
```

Then restart: `sudo systemctl restart capi`.

### Configuration Persistence

MQTT settings can also be configured from the web UI (see the MQTT Settings card). Changes made through the web UI are saved to `config.json` next to the binary (e.g. `/opt/capi/config.json`). CLI flags always take priority over the config file.

## HTTP API

Base URL: `http://<host>:8080/api`

All responses are JSON: `{"status": "success"|"error", "message": "...", "data": ...}`

### Devices

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/devices` | List active CEC devices. Add `?rescan=1` to force bus rescan. |
| GET | `/api/devices/{address}` | Get device info by logical address (0-15). |

### Power

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/power/on` | Power on TV (address 0). |
| POST | `/api/power/on/{address}` | Power on specific device. |
| POST | `/api/power/off` | Standby TV. |
| POST | `/api/power/off/{address}` | Standby specific device. |
| GET | `/api/power/status` | Get TV power status. |
| GET | `/api/power/status/{address}` | Get device power status. |

### Volume

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/volume/up` | Volume up (audio system). |
| POST | `/api/volume/up/{address}` | Volume up to specific device. |
| POST | `/api/volume/down` | Volume down. |
| POST | `/api/volume/down/{address}` | Volume down to specific device. |
| POST | `/api/volume/mute` | Toggle mute. |
| POST | `/api/volume/mute/{address}` | Toggle mute on specific device. |

### Source / HDMI

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/source/active` | Get current active source. |
| POST | `/api/source/{address}` | Switch to device by logical address. |
| POST | `/api/hdmi/{port}` | Switch TV to HDMI port (1-15). |

### Navigation

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/key` | Send key press. Body: `{"address": 4, "key": "select"}` or `{"address": 4, "keycode": 0}`. |

Supported key names: `up`, `down`, `left`, `right`, `select`, `enter`, `back`, `home`, `menu`, `play`, `pause`, `stop`.

### Raw CEC

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/command` | Send raw CEC command. Body: `{"initiator": 1, "destination": 0, "opcode": 143, "parameters": []}`. |

### System

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/topology` | Get CEC bus topology (own addresses, active ports, devices per port). |
| GET | `/api/audio/status` | Get volume level and mute state. |
| GET | `/api/logs` | Get recent CEC log messages. |
| GET | `/api/events` | Server-Sent Events stream of CEC bus events. |
| GET | `/api/health` | Health check (version, libcec info). |
| POST | `/api/update` | Trigger self-update from latest GitHub release. |
| GET | `/api/settings/mqtt` | Get MQTT configuration and connection status. |
| POST | `/api/settings/mqtt` | Update MQTT settings (persisted to `config.json`). |

### curl Examples

```bash
# Power on TV
curl -X POST http://localhost:8080/api/power/on

# Switch to HDMI 2
curl -X POST http://localhost:8080/api/hdmi/2

# Volume up
curl -X POST http://localhost:8080/api/volume/up

# Send navigation key
curl -X POST http://localhost:8080/api/key \
  -H "Content-Type: application/json" \
  -d '{"address": 4, "key": "select"}'

# List devices
curl http://localhost:8080/api/devices

# Power status
curl http://localhost:8080/api/power/status

# Raw CEC command (request vendor ID)
curl -X POST http://localhost:8080/api/command \
  -H "Content-Type: application/json" \
  -d '{"initiator": 1, "destination": 0, "opcode": 140}'

# Get MQTT settings
curl http://localhost:8080/api/settings/mqtt

# Update MQTT settings
curl -X POST http://localhost:8080/api/settings/mqtt \
  -H "Content-Type: application/json" \
  -d '{"broker":"tcp://localhost:1883","user":"","pass":"","prefix":"capi"}'
```

## MQTT

MQTT is opt-in. Pass `-mqtt-broker` to enable it, or configure it from the web UI's MQTT Settings card.

### Published Topics (CEC events)

Events from the CEC bus are published in real time:

| Topic | Payload | Description |
|-------|---------|-------------|
| `capi/event/power_change` | `{"address":0,"status":"on"}` | Device power state changed. |
| `capi/event/source_activated` | `{"address":4,"activated":true}` | Active source changed. |
| `capi/event/key_press` | `{"keycode":0,"duration":0}` | Remote key pressed. |
| `capi/event/command` | `{"initiator":0,"destination":1,"opcode":"0x90"}` | Raw CEC command seen on bus. |
| `capi/event/alert` | `{"alert":1,"param":0}` | CEC adapter alert. |

### Command Topics (MQTT to CEC)

Send commands by publishing to these topics:

| Topic | Payload | Description |
|-------|---------|-------------|
| `capi/command/power/on` | `0` (address, default TV) | Power on device. |
| `capi/command/power/off` | `0` (address) | Standby device. |
| `capi/command/volume/up` | (empty) | Volume up. |
| `capi/command/volume/down` | (empty) | Volume down. |
| `capi/command/volume/mute` | (empty) | Toggle mute. |
| `capi/command/source` | `4` (address) | Switch active source. |
| `capi/command/hdmi` | `2` (port) | Switch HDMI input. |
| `capi/command/key` | `{"address":4,"key":"select"}` | Send key press. |

All topics use the configurable prefix (default `capi`). Change with `-mqtt-prefix`.

### Home Assistant Example

```yaml
mqtt:
  button:
    - name: "TV Power On"
      command_topic: "capi/command/power/on"
      payload_press: "0"

    - name: "TV Power Off"
      command_topic: "capi/command/power/off"
      payload_press: "0"

    - name: "HDMI 1"
      command_topic: "capi/command/hdmi"
      payload_press: "1"

    - name: "HDMI 2"
      command_topic: "capi/command/hdmi"
      payload_press: "2"
```

## Web UI

Open `http://<device-ip>:8080` to access the built-in dashboard. Features:

- **Device list** with power status, vendor, HDMI port, and controls
- **Source/HDMI switching** with topology-aware port buttons
- **Volume control** (up, down, mute) with audio status display
- **Navigation pad** (D-pad, select, home, menu, back, media keys)
- **MQTT settings** configuration with connection status indicator
- **CEC log viewer** with color-coded levels
- **Live updates** via Server-Sent Events (no polling)
- **One-click update** when a new release is available

## Real-time Events (SSE)

Connect to `GET /api/events` for a Server-Sent Events stream of CEC bus activity:

```bash
curl -N http://localhost:8080/api/events
```

Events are JSON objects with `type`, `timestamp`, and `data` fields. Event types: `power_change`, `source_activated`, `key_press`, `command`, `alert`.

## Self-Update

### From the web UI

When a new release is available, an update badge appears in the header. Click it to update and restart automatically.

### From the CLI

```bash
sudo /opt/capi/capi -update
```

### From the API

```bash
curl -X POST http://localhost:8080/api/update
```

The update downloads the new binary and web UI from the latest GitHub release, then restarts the systemd service.

## Development

### Prerequisites

- Go 1.25+
- `libcec-dev`, `pkg-config` (build time)
- `libcec6`, `cec-utils` (runtime)
- A CEC adapter (Raspberry Pi built-in or USB Pulse-Eight)

### Building from Source

```bash
# Install build dependencies
sudo make setup

# Build
make build

# Build optimized release binary
make release

# Build with race detector
make dev
```

### Makefile Targets

| Target | Description |
|--------|-------------|
| `make build` | Build the binary |
| `make release` | Build optimized release binary |
| `make dev` | Build with race detector |
| `make install` | Install as systemd service |
| `make deploy` | Install, enable, and start service |
| `make uninstall` | Remove systemd service |
| `make clean` | Remove build artifacts |
| `make test` | Run tests |
| `make run` | Build and run locally |
| `make run-local` | Build and run on localhost only |
| `make status` | Show service status |
| `make logs` | Follow service logs |
| `make restart` | Restart service |
| `make deps` | Check build dependencies |

### Testing

```bash
# Run CEC package tests
make test

# Test with actual hardware
go run ./examples/example.go
```

## Troubleshooting

### No adapters found

```bash
# Check if adapter is connected
ls -la /dev/ttyACM* /dev/cec*

# Test with cec-client
cec-client -l

# Check permissions
sudo usermod -a -G dialout $USER
# Log out and back in
```

### Service won't start

```bash
# Check logs
sudo journalctl -u capi -n 50

# Test manually
sudo -u capi /opt/capi/capi

# Check adapter permissions
ls -la /dev/ttyACM0 /dev/cec0
```

### Connection fails

```bash
# Test CEC bus with cec-client
echo "scan" | cec-client -s -d 8

# Check libcec version
dpkg -l | grep libcec
```

## Go Package

The `cec` package can be used independently as a Go library:

```go
import "capi/cec"

conn, _ := cec.Open("My Device", cec.DeviceTypePlaybackDevice)
defer conn.Close()

adapters, _ := conn.FindAdapters()
conn.OpenAdapter(adapters[0].Path)

conn.PowerOn(cec.LogicalAddressTV)
conn.SwitchToHDMIPort(2)
conn.VolumeUp(true)

devices, _ := conn.GetAllDevices()
for _, dev := range devices {
    fmt.Printf("%s: %s\n", dev.OSDName, dev.PowerStatus.String())
}
```

See `examples/example.go` for comprehensive usage.

## License

MIT
