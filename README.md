# capi

HDMI-CEC control over HTTP and MQTT. A Rust service with full
[libcec](https://github.com/Pulse-Eight/libcec) bindings, a REST API,
real-time Server-Sent Events and WebSocket streams, MQTT integration, a web
dashboard, and over-the-air self-update. Designed for Raspberry Pi and ARM
SBCs.

## Quick Install

Run this on the target device (Raspberry Pi, etc.) as root. **Prefer a pinned
release** so installer behavior and binaries move together:

```bash
VERSION=v2.0.0   # pick the release you want
curl -sSL "https://raw.githubusercontent.com/LukasParke/capi/${VERSION}/install.sh" \
  | sudo VERSION=${VERSION} bash
```

The `main`-branch variant is a convenience (it always installs the latest
release with the newest installer):

```bash
curl -sSL https://raw.githubusercontent.com/LukasParke/capi/main/install.sh | sudo bash
```

The installer:

- Detects the host architecture (`arm64` / `armv6`) and the installed libcec
  ABI (`libcec6` on Debian 12 / Pi OS Bookworm; `libcec7` on Debian 13+ /
  Trixie) and downloads the matching release binary. Override with
  `FORCE_LIBCEC=6|7` if detection picks the wrong artifact.
- **Refuses to install unless the release ships `SHA256SUMS`**, verifies every
  downloaded artifact against it, and additionally refuses any artifact that
  is missing from the sums file.
- Backs up the previous binary to `/opt/capi/capi.bak` before swapping.
- Installs (or preserves, if you have `systemctl edit` overrides)
  `/etc/systemd/system/capi.service`, `/etc/udev/rules.d/99-cec.rules`, and
  `/etc/default/capi`, then runs `udevadm trigger` so already-plugged adapters
  pick up permissions without a replug.
- Gates the finish on a health loop (10 x 1s against `/api/health`); if the
  new binary never becomes healthy, the previous binary is **restored and the
  service restarted automatically**.

Once running, open `http://<device-ip>:8080`.

To **update**, run the same command again, use the web UI's update button, or
run `sudo /opt/capi/capi -update`.

### Verifying a download manually

```bash
VERSION=v2.0.0  # or whichever release
cd /tmp
curl -sSL -O "https://github.com/LukasParke/capi/releases/download/${VERSION}/SHA256SUMS"
curl -sSL -O "https://github.com/LukasParke/capi/releases/download/${VERSION}/capi-linux-arm64-libcec6"
sha256sum -c SHA256SUMS --ignore-missing
```

### Forcing a libcec variant

```bash
curl -sSL https://raw.githubusercontent.com/LukasParke/capi/main/install.sh \
  | sudo FORCE_LIBCEC=7 bash
```

## Features

- **Complete libcec bindings** with a drain-before-destroy discipline; no cgo
  crash windows (pure Rust FFI boundary)
- **REST API** for power, volume, source/HDMI switching, navigation keys, raw
  CEC commands, and a low-level dev console
- **Optional bearer-token authentication** with CSRF/same-origin defenses
- **MQTT bridge** with a retained availability topic + LWT (opt-in)
- **Web dashboard** with live device status, remote control, and one-click
  update
- **Server-Sent Events + WebSocket** for real-time CEC bus monitoring
- **Self-update** from GitHub releases with checksum verification, semver
  compare (never downgrades), single-flight locking, and `.bak` rollback
- **Systemd service** with a hardened sandbox and udev rules
- **Automatic adapter detection** (built-in HDMI CEC and USB Pulse-Eight)
- Prometheus metrics at `/metrics`

## Authentication

Auth is **off by default**. Enable it by passing a token:

```bash
# /etc/default/capi
CAPI_EXTRA_FLAGS=-token s3cr3t
```

or `sudo systemctl edit capi.service`, or directly: `./capi -token s3cr3t`.
When a token is configured:

- Every `/api` route **except `GET /api/health`** (and `GET /metrics`)
  requires the token. Accepted: `Authorization: Bearer <token>`,
  `X-Auth-Token: <token>`, `?key=<token>`, or the `capi_token` cookie set by
  the UI login page.
- All mutating `/ui` actions require the token too, and cookie-authenticated
  mutations additionally require a same-origin request (Origin host match or
  `Sec-Fetch-Site: same-origin`).
- A Host-header allowlist (bind host, `localhost`, request IP literal) rejects
  DNS-rebinding attempts.

### curl examples

```bash
# Without the token: 401
curl -i http://localhost:8080/api/devices

# Bearer header
curl -H "Authorization: Bearer s3cr3t" http://localhost:8080/api/devices

# Header equivalent
curl -H "X-Auth-Token: s3cr3t" http://localhost:8080/api/devices

# Query parameter (handy for SSE/WS clients)
curl -N "http://localhost:8080/api/events?key=s3cr3t"

# Health stays open
curl http://localhost:8080/api/health
```

## Configuration

### CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `-bind` | `:8080` | Bind address (`:8080` for all interfaces, `localhost:8080` for local only) |
| `-name` | `CEC HTTP Bridge` | CEC device name on the bus |
| `-adapter` | (auto-detect) | CEC adapter path (e.g. `/dev/cec0`, `/dev/ttyACM0`) |
| `-token` | (auth disabled) | Bearer token; when set, all `/api` except `/api/health` require it |
| `-mqtt-broker` | (disabled) | MQTT broker URL (e.g. `tcp://localhost:1883`). Empty disables MQTT. |
| `-mqtt-user` | | MQTT username |
| `-mqtt-pass` | | MQTT password |
| `-mqtt-prefix` | `capi` | MQTT topic prefix |
| `-version` | | Print version and exit |
| `-update` | | Check for updates and install the latest release |

The version string is baked in at build time from the `CAPI_VERSION` env var
(`option_env!`); release CI passes the tag through.

### Systemd Service

The install script sets up `/etc/systemd/system/capi.service` running as the
`capi` system user with `ProtectSystem=strict`, `SystemCallFilter=
@system-service`, empty `CapabilityBoundingSet`, `ProtectProc=invisible`, and
friends. The simplest way to pass extra CLI flags is `/etc/default/capi`:

```bash
# /etc/default/capi
CAPI_EXTRA_FLAGS=-mqtt-broker tcp://localhost:1883 -mqtt-prefix capi
```

Then `sudo systemctl restart capi`. The unit's `ExecStart` is:

```
ExecStart=/opt/capi/capi -bind :8080 -name "CEC HTTP Bridge" $CAPI_EXTRA_FLAGS
```

For a full unit override, use `sudo systemctl edit capi.service`. The
installer leaves your unit untouched if it detects a `capi.service.d/`
override directory.

### Configuration Persistence

MQTT settings and the CEC mode can also be configured from the web UI or the
API (`/api/settings/mqtt`, `/api/dev/mode`). Changes are persisted to
`config.json` next to the binary (`/opt/capi/config.json`) — the in-memory
config only advances after the disk write succeeds. CLI flags always take
priority over the config file. A `config.json` that fails to parse refuses to
boot (the broken file is quarantined, never silently reset).

## HTTP API

Base URL: `http://<host>:8080/api`

All responses are JSON envelopes:
`{"status": "success"|"error", "message": "...", "data": ...}` (`message` and
`data` omitted when empty/absent). Full machine-readable spec:
[`openapi.yaml`](openapi.yaml).

### Devices

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/devices` | List CEC devices. Default: immediate cache. `?wait=N` (≤10s) waits for a fresher scan. `?live=1`/`?rescan=1` force a full reconcile. Each entry carries `discovery` (`active`/`polled`/`observed`), `polled_at`, `first_seen_at`, `last_seen_at`, and passive `observed_*` fields. |
| GET | `/api/devices/{address}` | Get device info by logical address (0-15). |

### Bus

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/bus/state` | Cached steward snapshot: devices, `active_source`, `stale`, `scan_in_progress`, optional `recent_frames`. |
| POST | `/api/bus/scan` | Queue a deep bus scan (202, async). Poll `/api/bus/state`. |
| GET | `/api/bus/frames` | Recent captured frames (ring buffer with monotonic `seq`). |

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
| POST | `/api/volume/up` | Volume up (TV's system audio path). |
| POST | `/api/volume/up/{address}` | Volume up to specific device (e.g. 5 = audio system). |
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
| POST | `/api/key` | Send key press. Body: `{"address": 4, "key": "select"}`. **Keycode 0 is rejected** — send `{"key": "select"}` for Select. |

Supported key names (59): `select`, `up`, `down`, `left`, `right`,
`right_up`, `right_down`, `left_up`, `left_down`, `root_menu`, `home`,
`setup_menu`, `menu`, `contents_menu`, `favorite_menu`, `exit`, `back`,
`enter`, `clear`, `0`-`9`, `dot`, `channel_up`, `channel_down`,
`previous_channel`, `sound_select`, `input_select`, `display_information`,
`help`, `page_up`, `page_down`, `power`, `volume_up`, `volume_down`, `mute`,
`play`, `stop`, `pause`, `record`, `rewind`, `fast_forward`, `eject`,
`forward`, `backward`, `angle`, `subpicture`, `f1_blue`, `f2_red`,
`f3_green`, `f4_yellow`, `f5`. Machine-readable: `GET /api/dev/keys` or
[`openapi.yaml`](openapi.yaml).

### Raw CEC

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/command` | Send raw CEC command. Body: `{"initiator": 1, "destination": 0, "opcode": 143, "parameters": []}`. |

### System

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/topology` | CEC bus topology, served from the cached steward snapshot (refresh is async). |
| GET | `/api/audio/status` | Volume level and mute state. |
| GET | `/api/logs` | Recent log messages (levels: `CEC`, `APP`, `ERROR`, `WARN`, `INFO`, `DEBUG`). |
| GET | `/api/events` | SSE stream (`text/event-stream`). |
| GET | `/api/events/ws` | WebSocket stream (same payloads). |
| GET | `/api/health` | Health check (version, libcec info). Never requires auth. |
| GET | `/metrics` | Prometheus metrics (`text/plain`). Never requires auth. |
| POST | `/api/update` | Trigger self-update (checksum-verified, single-flight). |
| GET | `/api/settings/mqtt` | Get MQTT configuration and connection status. |
| POST | `/api/settings/mqtt` | Update MQTT settings (persisted to `config.json`). |

### Dev console

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/dev/mode` | Current CEC mode (`passive`/`monitor_only`) + adapter state. |
| POST | `/api/dev/mode` | `{"mode": "passive" \| "monitor_only"}`; persists and reconnects. |
| POST | `/api/dev/probe` | Run Give* probes (`kind`: power/osd/vendor/cec_version/physical/audio/sam/menu/all; `observe_ms` ≤ 5000). |
| POST | `/api/dev/send_key` | Blind key send (`hold_ms` ≤ 2000, `repeat` ≤ 32, keycode 0 rejected). |
| POST | `/api/dev/send_opcode` | Blind raw opcode send (`params_hex`). |
| POST | `/api/dev/run_strategies` | Run the strategy chain (single-flight per connection). |
| POST | `/api/dev/save_strategy` | Save a winning strategy per vendor. |
| GET | `/api/dev/actions` | List registry actions. |
| GET | `/api/dev/keys` | Key name → keycode table. |
| GET | `/api/dev/opcodes` | Known CEC opcodes. |

### Web UI routes

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/` | Dashboard |
| GET | `/settings` | Settings page |
| GET | `/dev` | Dev console |
| GET | `/ui/static/*` | Embedded assets |
| GET | `/ui/fragment/{name}` | `bus_banner`, `devices`, `device_power`, `mqtt_panel`, `health`, `topology_hdmi`, `volume_panel`, `nav_panel`, `source_panel`, `logs` |
| POST | `/ui/action/{name}` | `deep_scan`, `power_on`, `power_off`, `volume_up`, `volume_down`, `volume_mute`, `set_source`, `hdmi`, `nav_key`, `mqtt_save` |
| GET | `/ui/dev/fragment/{name}` | `banner`, `devices`, `trace` |
| POST | `/ui/dev/action/{name}` | `mode`, `probe`, `send_key`, `send_opcode`, `run_strategies`, `save_strategy` |

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

# List devices (immediate cache)
curl http://localhost:8080/api/devices

# Wait up to 5s for a fresher scan
curl "http://localhost:8080/api/devices?wait=5"

# Power status
curl http://localhost:8080/api/power/status

# Raw CEC command (request vendor ID)
curl -X POST http://localhost:8080/api/command \
  -H "Content-Type: application/json" \
  -d '{"initiator": 1, "destination": 0, "opcode": 140}'

# Get / update MQTT settings
curl http://localhost:8080/api/settings/mqtt
curl -X POST http://localhost:8080/api/settings/mqtt \
  -H "Content-Type: application/json" \
  -d '{"broker":"tcp://localhost:1883","user":"","pass":"","prefix":"capi"}'

# Prometheus metrics
curl http://localhost:8080/metrics
```

## MQTT

MQTT is opt-in. Pass `-mqtt-broker` to enable it, or configure it from the web
UI's MQTT Settings card.

### Availability topic (NEW)

| Topic | Payload | Description |
|-------|---------|-------------|
| `capi/status` | `online` / `offline` | Retained availability. Published as `online` on connect; the broker publishes the **LWT `offline`** if capi disconnects uncleanly. Subscribe here to gate automations. |

### Published Topics (CEC events)

Events from the CEC bus are published in real time (retained last-state per
topic, so late subscribers see the current value):

| Topic | Payload | Description |
|-------|---------|-------------|
| `capi/event/power_change` | `{"address":0,"status":"on"}` | Device power state changed. |
| `capi/event/source_activated` | `{"address":4,"activated":true}` | Active source changed. |
| `capi/event/key_press` | `{"keycode":0,"duration":0}` | Remote key pressed. |
| `capi/event/command` | `{"initiator":0,"destination":1,"opcode":"0x90"}` | Raw CEC command seen on bus. |
| `capi/event/alert` | `{"alert":1,"param":0}` | CEC adapter alert. |
| `capi/event/devices_changed` | `{"reason":"bus_topology","logical_addresses":[0,1,4]}` | After a bus rescan. |
| `capi/event/configuration_changed` | `{"device_name":"CEC HTTP Bridge"}` | libCEC client configuration changed. |
| `capi/event/adapter_state` | `{"state":"connected"}` or `{"state":"disconnected"}` | HDMI/USB adapter session up or down. |

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

All topics use the configurable prefix (default `capi`). Change with
`-mqtt-prefix`.

### Home Assistant Example

```yaml
mqtt:
  binary_sensor:
    - name: "capi availability"
      state_topic: "capi/status"
      payload_on: "online"
      payload_off: "offline"

  button:
    - name: "TV Power On"
      command_topic: "capi/command/power/on"
      payload_press: "0"

    - name: "TV Power Off"
      command_topic: "capi/command/power/off"
      payload_press: "0"
```

## Web UI

Open `http://<device-ip>:8080` to access the built-in dashboard. Features:

- **Device list** with power status, vendor, HDMI port, discovery provenance,
  and controls
- **Source/HDMI switching** with topology-aware port buttons
- **Volume control** (up, down, mute) with audio status display
- **Navigation pad** (D-pad, select, home, menu, back, media keys)
- **MQTT settings** configuration with connection status indicator
- **Log viewer** with color-coded levels
- **Live updates** via Server-Sent Events (no polling)
- **One-click update** when a new release is available
- **Login page** when token auth is enabled (sets the `capi_token` cookie)

## Real-time Events (SSE / WebSocket)

Connect to `GET /api/events` (SSE, `text/event-stream`) or
`GET /api/events/ws` (WebSocket):

```bash
curl -N http://localhost:8080/api/events
```

Events are JSON objects with `type`, `timestamp`, and `data` fields. Event
types: `power_change`, `source_activated`, `key_press`, `command`, `alert`,
`devices_changed`, `configuration_changed`, `adapter_state`. The service
debounces bus rescans on topology-related traffic and emits `devices_changed`
after a rescan so clients can refresh the device list (the web UI does this
automatically).

## Update & Rollback

### From the web UI

When a new release is available, an update badge appears in the header. Click
it to update and restart automatically.

### From the CLI

```bash
sudo /opt/capi/capi -update
```

### From the API

```bash
curl -X POST http://localhost:8080/api/update
```

### What an update does

1. Fetches the release manifest for the **runtime-linked libcec ABI** and
   picks the matching `capi-linux-*-libcecN` asset.
2. Downloads the binary **and `SHA256SUMS`**, verifies the checksum before
   touching anything (missing sums = aborted update).
3. Compares versions with semver: **never downgrades**, and equal versions
   are no-ops.
4. Uses a unique temp file + single-flight lock (no fixed `.tmp` race).
5. Keeps the previous binary at `/opt/capi/capi.bak`.
6. Restarts the service via systemd. If the restart requires privileges the
   service user doesn't have (polkit), the failure is **reported honestly**
   instead of silently doing nothing.

### Manual rollback

```bash
sudo systemctl stop capi
sudo mv /opt/capi/capi.bak /opt/capi/capi
sudo systemctl start capi
```

`install.sh` performs the same backup automatically and restores it itself if
the post-install health gate fails.

## Development

### Prerequisites

- Rust (rustup, pinned toolchain; see `dockerfiles/builder.Dockerfile`)
- `libcec-dev`, `libp8-platform-dev`, `libudev-dev`, `pkg-config` (build time)
- `libcec6`/`libcec7`, `cec-utils` (runtime)
- A CEC adapter (Raspberry Pi built-in or USB Pulse-Eight)

### Building from Source

```bash
sudo apt-get install -y pkg-config libcec-dev libp8-platform-dev libudev-dev cec-utils
cargo build --release --locked   # native build -> target/release/capi
CAPI_VERSION=v0.0.0-dev cargo build --release --locked  # with a version string
cargo test
cargo clippy -- -D warnings
```

### Cross-builds (dev machine → ARM)

```bash
# First time on a Docker host that needs QEMU for non-native arch:
docker run --privileged --rm tonistiigi/binfmt --install all

scripts/cross-build.sh arm64 6    # -> dist/capi-linux-arm64-libcec6
scripts/cross-build.sh armv6 6    # -> dist/capi-linux-armv6-libcec6
scripts/cross-build.sh arm64 7    # -> dist/capi-linux-arm64-libcec7
```

### Iteration loop: dev machine → local Pi

```bash
cp .env.example .env   # fill in SSH_USER / SSH_IP (and SSH_PASSWORD if not using keys)
make push-pi           # cross-build (Docker), scp, restart, curl /api/health
make logs-pi           # tail journalctl -u capi.service on the Pi
FOLLOW=1 make logs-pi  # follow mode
make deploy-pi         # no-Docker fallback: rsync source, build on the Pi
```

The make targets delegate to `scripts/push-pi.sh`, `scripts/deploy-pi.sh`,
and `scripts/pi-logs.sh`. `push-pi` swaps the binary (with `.bak` backup);
the systemd unit, udev rule, and `/etc/default/capi` are managed by
`install.sh` (run once on the Pi). Passwords travel via `sshpass -e` (SSHPASS
env), never on a command line. See `.env.example` for the full knob list,
including `CAPI_KNOWN_HOSTS` for pinning the Pi's host key.

### Testing

```bash
cargo test          # unit + integration tests
bash test.sh        # against a running instance (set API_URL / CAPI_TOKEN)
```

## Releases

A push to `main` (or a manual `workflow_dispatch`) auto-bumps the patch
component of the latest `v*` tag, tags the new commit, runs the cross-build
matrix in CI, and publishes:

- `capi-linux-arm64-libcec6` (Raspberry Pi OS Bookworm / Debian 12)
- `capi-linux-arm64-libcec7` (Raspberry Pi OS Trixie / Debian 13+)
- `capi-linux-armv6-libcec6` (Pi 1 / Zero W on Bookworm)
- `install.sh`, `capi.service`, `99-cec.rules`, `SHA256SUMS`
- A changelog generated from `git log` since the previous tag

CI hardening: per-job minimum permissions, actions pinned to commit SHAs,
`env:` indirection for every interpolated value in `run:` blocks,
sha256-verified pinned Rust toolchain, and a release concurrency group.

For minor or major bumps, push the tag yourself before merging:

```bash
git tag v2.1.0 && git push origin v2.1.0   # workflow uses this tag verbatim
```

Or use the workflow's manual dispatch with `bump=minor` / `bump=major`.

## Migrating from the Go version

The Rust rewrite is a drop-in replacement:

- **Same binary name and install path** (`/opt/capi/capi`), same
  `config.json`, same HTTP API and event wire format, same systemd unit name.
- Your existing `/opt/capi/config.json`, `/etc/default/capi`, and
  `systemctl edit` overrides carry over unchanged.
- Install as usual with `install.sh`; it detects the old binary and performs
  a normal update (backup + health gate + rollback).
- New capabilities you gain: optional token auth (`-token`), the MQTT
  availability topic (`{prefix}/status`), `/metrics`, `/api/dev/*`, and
  checksum-verified self-update. Nothing you depended on was removed.

## Troubleshooting

### No adapters found

```bash
# Check if adapter is connected
ls -la /dev/ttyACM* /dev/cec*

# Test with cec-client
cec-client -l

# Check permissions (the udev rule grants group capi)
ls -l /dev/ttyACM0 /dev/cec0
```

The installer runs `udevadm trigger`, so adapters plugged in before the
install get the new permissions without a replug. If you installed the rules
manually, replug once or run `sudo udevadm trigger`.

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

CEC only sees devices that share the **same switched path** as your Pi's HDMI
input. A streamer on **another** HDMI port on the projector may not
participate in CEC, or the projector may not bridge that port onto the same
logical bus.

- Call **`GET /api/devices?rescan=1`** so the service runs a full bus rescan
  and POLLs every logical address (0–14); devices that ACK but were omitted
  from libcec's active list are then listed.
- For volume, **`POST /api/volume/up`** uses libcec's default audio routing
  (often the TV or ARC path). If the device you want is listed at a specific
  logical address (often **5** for Audio System or **4** for Playback 1), use
  **`POST /api/volume/up/5`**.
- If a device never appears even after `rescan=1`, it likely does not speak
  CEC on that input, or the display is not bridging CEC between ports
  (hardware limitation, not something software can fix).

## License

MIT
