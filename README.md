# capi

HDMI-CEC control over HTTP and MQTT. A Go service with full [libcec](https://github.com/Pulse-Eight/libcec) bindings, a REST API, real-time Server-Sent Events, MQTT integration, a web dashboard, and over-the-air self-update. Designed for Raspberry Pi and ARM SBCs.

## Quick Install

Run this on the target device (Raspberry Pi, etc.) as root:

```bash
curl -sSL https://raw.githubusercontent.com/LukasParke/capi/main/install.sh | sudo bash
```

The installer:

- Detects the host architecture (`arm64` / `armv6`) and the installed libcec ABI (`libcec6` on Debian 12 / Pi OS Bookworm; `libcec7` on Debian 13+ / Trixie) and downloads the matching release binary.
- Verifies every download against the release's `SHA256SUMS`.
- Atomically swaps the binary, installs (or preserves) `/etc/systemd/system/capi.service`, the `/etc/udev/rules.d/99-cec.rules`, and `/etc/default/capi` (for extra CLI flags), then starts the service.

Once running, open `http://<device-ip>:8080`.

To **update**, run the same command again, or use the web UI's update button, or run `sudo /opt/capi/capi -update`.

### Verifying a download manually

```bash
VERSION=v0.1.0  # or whichever release
cd /tmp
curl -sSL -O "https://github.com/LukasParke/capi/releases/download/${VERSION}/SHA256SUMS"
curl -sSL -O "https://github.com/LukasParke/capi/releases/download/${VERSION}/capi-linux-arm64-libcec6"
sha256sum -c SHA256SUMS --ignore-missing
```

### Pinning a specific release

```bash
curl -sSL https://raw.githubusercontent.com/LukasParke/capi/main/install.sh \
  | sudo VERSION=v0.1.0 bash
```

### Forcing a libcec variant

The installer's libcec ABI detection can be overridden if it picks the wrong artifact:

```bash
curl -sSL .../install.sh | sudo FORCE_LIBCEC=7 bash
```

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

The install script sets up `/etc/systemd/system/capi.service` running as the `capi` system user with kernel-namespace, `ProtectSystem=strict`, and friends.

The simplest way to pass extra CLI flags is via `/etc/default/capi`, which the unit reads via `EnvironmentFile=`:

```bash
# /etc/default/capi
CAPI_EXTRA_FLAGS=-mqtt-broker tcp://localhost:1883 -mqtt-prefix capi
```

Then `sudo systemctl restart capi`. The unit's `ExecStart` is:

```
ExecStart=/opt/capi/capi -bind :8080 -name "CEC HTTP Bridge" $CAPI_EXTRA_FLAGS
```

For a full unit override, use `sudo systemctl edit capi.service` and replace `ExecStart=` with two lines (the first empty to reset, the second with your full command). The installer leaves your unit untouched if it detects a `capi.service.d/` override directory.

### Configuration Persistence

MQTT settings can also be configured from the web UI (see the MQTT Settings card). Changes made through the web UI are saved to `config.json` next to the binary (e.g. `/opt/capi/config.json`). CLI flags always take priority over the config file.

## HTTP API

Base URL: `http://<host>:8080/api`

All responses are JSON: `{"status": "success"|"error", "message": "...", "data": ...}`

### Devices

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/devices` | List CEC devices. Without `rescan`, merges **POLL** probes for common logical roles so sinks (playback/audio) missing from libcec’s active mask still appear. `?rescan=1` forces a bus rescan and **full** POLL sweep (0–14). |
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
| POST | `/api/volume/up` | Volume up via libcec (targets the TV’s **system audio** path). If volume should go to another device (e.g. Chromecast / Google TV), use `/api/volume/up/{address}` with that device’s logical address. |
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
| GET | `/api/events` | SSE stream: power/source/key/command/alert plus `devices_changed` (debounced rescan), `adapter_state`, `configuration_changed`. |
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
| `capi/event/devices_changed` | `{"reason":"bus_topology","logical_addresses":[0,1,4]}` | After a bus rescan; device list or topology may have changed. |
| `capi/event/configuration_changed` | `{"device_name":"CEC HTTP Bridge"}` | libCEC client configuration changed. |
| `capi/event/adapter_state` | `{"state":"connected"}` or `{"state":"disconnected"}` | HDMI/USB adapter session up or down (reconnect loop). |

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

Events are JSON objects with `type`, `timestamp`, and `data` fields. Event types: `power_change`, `source_activated`, `key_press`, `command`, `alert`, `devices_changed`, `configuration_changed`, `adapter_state`. The service debounces bus rescans on topology-related traffic and emits `devices_changed` after `RescanDevices()` so clients can refresh the device list (the web UI does this automatically).

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
sudo make setup        # apt install pkg-config libcec-dev libp8-platform-dev cec-utils
make build             # native build -> ./capi-server
make release           # optimized native build (-s -w)
make dev               # native build with -race
make test              # go test -race ./cec ./capi
make bench             # benchmark suite
```

### Iteration loop: dev machine → local Pi

For working against a Raspberry Pi over SSH, keep iteration time short by cross-building on your dev machine and pushing only the binary:

```bash
cp .env.example .env   # fill in SSH_USER / SSH_IP (and SSH_PASSWORD if not using keys)

# First time on a Docker host that needs QEMU for non-native arch:
docker run --privileged --rm tonistiigi/binfmt --install all

make push-pi           # cross-build (Docker), scp, restart, curl /api/health
make logs-pi           # tail journalctl -u capi.service on the Pi
make logs-pi-follow    # follow mode
```

`push-pi` only swaps the binary; the systemd unit, udev rule, and `/etc/default/capi` are managed by `install.sh` (run once on the Pi). The cross-build picks `arm64` + `libcec6` by default; override with `PI_ARCH=armv6` and/or `PI_LIBCEC=7` either in `.env` or on the make line.

For environments without Docker, `make deploy-pi` falls back to the slower path of rsync-then-build-on-Pi.

### Releases

A push to `main` (or a manual `workflow_dispatch`) auto-bumps the patch component of the latest `v*` tag, tags the new commit, runs the cross-build matrix in CI, and publishes:

- `capi-linux-arm64-libcec6` (Raspberry Pi OS Bookworm / Debian 12)
- `capi-linux-arm64-libcec7` (Raspberry Pi OS Trixie / Debian 13+)
- `capi-linux-armv6-libcec6` (Pi 1 / Zero W on Bookworm)
- `install.sh`, `capi.service`, `99-cec.rules`, `SHA256SUMS`
- A changelog generated from `git log` since the previous tag

For minor or major bumps, push the tag yourself before merging:

```bash
git tag v0.5.0 && git push origin v0.5.0   # workflow uses this tag verbatim
```

Or use the workflow's manual dispatch with `bump=minor` / `bump=major`.

### Makefile Targets

| Target | Description |
|--------|-------------|
| `make build` / `release` / `dev` | Native builds |
| `make test` / `bench` | Tests, benchmarks |
| `make cross-build` | ARM cross-compile (PI_ARCH/PI_LIBCEC env) |
| `make push-pi` | Cross-build + scp + restart on Pi (`.env`) |
| `make logs-pi` / `logs-pi-follow` | Tail Pi journalctl |
| `make deploy-pi` | Slow fallback: source-build on Pi |
| `make install` / `deploy` | On-target install + start (run on the Pi) |
| `make uninstall` | Remove the service (leaves `/etc/default/capi`) |
| `make status` / `logs` / `restart` | Local systemd helpers |

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

### Missing devices or volume on a projector / switch

CEC only sees devices that share the **same switched path** as your Pi’s HDMI input. A Google Home or streamer on **another** HDMI port on the projector may not participate in CEC, or the projector may not merge that port into the same logical bus.

- Call **`GET /api/devices?rescan=1`** so the service runs a full bus rescan and **POLL**s every logical address (0–14); devices that ACK but were omitted from libcec’s active list are then listed.
- For volume, **`POST /api/volume/up`** uses libcec’s default audio routing (often the TV or ARC path). If the device you want is listed at a specific logical address (often **5** for Audio System or **4** for Playback 1), use **`POST /api/volume/up/5`** (replace `5` with the address from the device list).
- If a device never appears even after `rescan=1`, it likely does not speak CEC on that input, or the display is not bridging CEC between ports (hardware limitation, not something software can fix).

## Go Package

The `cec` package is published at `github.com/LukasParke/capi/cec` and can be used independently as a Go library. It wraps libcec via cgo with a typed events channel and internal serialization.

```bash
go get github.com/LukasParke/capi/cec
```

```go
import "github.com/LukasParke/capi/cec"

conn, _ := cec.Open("My Device", cec.DeviceTypePlaybackDevice)
defer conn.Close()

// Drain async events on a goroutine; channel is closed by Close().
go func() {
    for ev := range conn.Events() {
        switch ev.Kind {
        case cec.EventKeyPress:
            fmt.Println("key:", ev.Key.Key)
        case cec.EventCommand:
            fmt.Println("cmd:", ev.Command.Opcode)
        }
    }
}()

adapters, _ := conn.FindAdapters()
conn.OpenAdapter(adapters[0].Path)

conn.PowerOn(cec.LogicalAddressTV)
conn.SwitchToHDMIPort(2)
conn.VolumeUp(true)

devices, _ := conn.GetAllDevices(2 * time.Second)
for _, dev := range devices {
    fmt.Printf("%s: %s\n", dev.OSDName, dev.PowerStatus)
}
```

Every `*cec.Connection` method is safe to call from multiple goroutines; libcec calls are serialized inside the package. `Events()` fires on libcec threads and never blocks a Go goroutine — slow consumers see drops instead of pressure.

See `examples/example.go` for comprehensive usage.

## License

MIT
