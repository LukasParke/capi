# capi review findings (defect list for the Rust rebuild)

Every item below must be closed by construction in the Rust service or by the
ops rewrite. Wire/API/config compatibility is preserved; defects are not.

## Route table (authoritative for server + openapi + UI)

GET  /                          dashboard
GET  /settings                  settings page
GET  /dev                       dev console
GET  /ui/static/*               embedded assets
GET  /ui/fragment/{bus_banner,devices,device_power,mqtt_panel,health,topology_hdmi,volume_panel,nav_panel,source_panel,logs}
POST /ui/action/{deep_scan,power_on,power_off,volume_up,volume_down,volume_mute,set_source,hdmi,nav_key,mqtt_save}
GET  /ui/dev/fragment/{banner,devices,trace}
POST /ui/dev/action/{mode,probe,send_key,send_opcode,run_strategies,save_strategy}
GET  /api/devices[?live&rescan&wait]        GET /api/devices/{address}
GET  /api/bus/state | POST /api/bus/scan | GET /api/bus/frames
GET  /api/topology
POST /api/power/on[/{address}] | POST /api/power/off[/{address}]
GET  /api/power/status[/{address}]
POST /api/volume/up|down|mute[/{address}]
GET  /api/source/active         POST /api/source/{address}   POST /api/hdmi/{port}
GET  /api/audio/status
POST /api/key                   POST /api/command
GET  /api/logs                  GET /api/events (SSE)        GET /api/events/ws
GET  /api/health                GET /metrics                 POST /api/update
GET|POST /api/settings/mqtt
GET|POST /api/dev/mode          POST /api/dev/probe
POST /api/dev/send_key          POST /api/dev/send_opcode
POST /api/dev/run_strategies    POST /api/dev/save_strategy
GET  /api/dev/actions           GET /api/dev/keys            GET /api/dev/opcodes

## Defects to fix in Rust code

1. Self-update broken: legacy asset names (must use -libcecN suffix chosen by
   RUNTIME linked ABI via libcec server version), no checksum verify (must
   fetch SHA256SUMS + verify before swap), naive version equality (semver
   compare, never downgrade silently), restart as unprivileged user fails
   silently (report honestly; support token-less polkit failure message),
   fixed .tmp path race (unique temp + single-flight), no rollback (keep .bak).
2. No authn/authz/CSRF: optional bearer token (config auth_token / -token);
   when set, all /api except /api/health + all mutating /ui actions require it
   (Authorization: Bearer | X-Auth-Token | ?key= | cookie capi_token). UI login
   page sets cookie. Mutating requests with cookie auth require same-origin
   (Origin host match or Sec-Fetch-Site same-origin). Host header allowlist
   defense against DNS rebinding: when token set, reject requests whose Host
   is not in {bind host, localhost, request IP literal}. Empty Origin allowed
   only for non-browser clients (no Sec-Fetch header present).
3. cgo crash window: N/A in Rust but keep drain-before-destroy discipline.
4. Transmit param bounds (>64) + size truncation: validate.
5. diffFrames ring-wrap misclassification: frame ring entries carry monotonic
   sequence numbers; diff = entries with seq > pre_max_seq. Never index-diff.
6. Unbounded observe_ms/hold_ms/ObserveOverrideMs: clamp (observe <= 5000ms,
   hold <= 2000ms, repeat <= 32) and derive contexts from request.
7. install.sh silent errexit deaths, missing rollback, missing udevadm trigger,
   weak health gate: ops contract.
8. CI workflow injection, global permissions, mutable action tags, unverified
   toolchain downloads, no concurrency group: ops contract.
9. test.sh ((x++)) errexit bug: ops contract.
10. Blocking CEC calls on request paths: topology building moves behind the
    steward snapshot (topology served from cached snapshot; refresh async).
    Direct conn calls from handlers limited to fast transmits; long probes run
    with spawn_blocking + request-derived timeouts.
11. runAction ignores cancellation: derive timeout ctx from request where
    available; hard cap 5s retained.
12. Concurrent strategy runs interleave frames: single-flight mutex around
    Registry::run per connection.
13. Config parse errors silently reset: refuse at boot, quarantine file.
14. In-memory config mutated before save confirmed: update-and-persist under
    one lock; memory only advances when disk write succeeded.
15. MQTT: events dropped while disconnected (buffer last-state per topic with
    retained publishes; availability topic prefix/status retained online/offline
    + LWT); command handlers must not block the event loop (spawn).
16. Status codes: address 15 -> 400 not 500; errors.Is equivalents via
    typed error enums; keycode 0 rejected with explicit message (parity).
17. openapi drift: regenerate (ops contract).
18. gofmt/nits class: rustfmt + clippy -D warnings clean; no dead code;
    strings.Title equivalent via proper casing helper; X-Request-ID sanitized
    (hex only, ignore client-provided non-hex).

## Envelope

{"status":"success"|"error","message":"...","data":...} — omit empty message,
omit absent data. Async acceptance returns success with data.accepted=true.

## Events (SSE/WS/MQTT wire format — unchanged)

{"type":"power_change|source_activated|key_press|command|alert|
devices_changed|configuration_changed|adapter_state",
 "timestamp":"RFC3339","data":{...}}
MQTT topics: {prefix}/event/{type}, commands {prefix}/command/... (same payloads),
NEW: {prefix}/status retained "online"/"offline" (LWT offline).
