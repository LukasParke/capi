# Contract: capi front end (templates + CSS + JS)

Backend: axum + askama (compile-time templates, auto-escaped) + htmx (vendored at /ui/static/htmx.min.js). Context structs live in src/ui_ctx.rs — template field access must match those EXACTLY (askama compiles against them). All POST targets require same-origin; when auth token is set they also need the cookie (set by /login).

## Routes you target (do not invent others)

Pages: GET / (dashboard), GET /settings, GET /dev, GET /login
Fragments (GET): /ui/fragment/bus_banner, /ui/fragment/devices, /ui/fragment/device_power, /ui/fragment/mqtt_panel, /ui/fragment/health, /ui/fragment/topology_hdmi, /ui/fragment/volume_panel, /ui/fragment/nav_panel, /ui/fragment/source_panel, /ui/fragment/logs
UI actions (POST, form-encoded, respond with HTML fragments or hx-trigger headers):
  /ui/action/deep_scan, power_on, power_off, volume_up, volume_down, volume_mute, set_source (form: addr), hdmi (form: port), nav_key (form: key, addr optional), mqtt_save (form: broker, user, pass, prefix)
Dev fragments (GET): /ui/dev/fragment/banner, /ui/dev/fragment/devices, /ui/dev/fragment/trace
Dev actions (POST form-encoded): /ui/dev/action/mode (form: monitor_only=0|1), probe (form: addr, kind, observe_ms), send_key (addr, key, hold_ms, repeat), send_opcode (dest, opcode, params_hex), run_strategies (action, target, observe_ms, all_strategies), save_strategy (vendor, action, strategy)
Streams: GET /api/events (SSE, named events: power_change, source_activated, key_press, command, alert, devices_changed, configuration_changed, adapter_state; data is JSON {type,timestamp,data}), GET /api/events/ws (WebSocket pushing OOB HTML fragments for panels + feed lines)
JSON APIs for settings/dev pages: GET/POST /api/settings/mqtt, GET /api/health, GET /api/logs, GET /api/bus/frames, GET /api/bus/state, GET /api/topology, POST /api/update, GET/POST /api/dev/mode, GET /api/dev/actions, GET /api/dev/keys, GET /api/dev/opcodes

## Template inventory (askama, templates/)

shell.html          base layout: <head> (css, htmx, app.js), nav (Dashboard/Settings/Dev), theme toggle, toast container. Blocks: title, content.
login.html          extends shell; token entry form POSTs to /login (form field name: token). Error message block on bad token.
dashboard.html      extends shell; composes fragments via {% include %}: bus_banner, devices, source_panel (HDMI strip), volume_panel, nav_panel, logs feed (live), quick controls row.
bus_banner.html     BusBannerData — status pills: adapter (Ready/Offline), monitoring on/off, scan spinner when scan_in_progress, stale badge, last full scan time, device count. When !cec_ready show explanatory banner: "No CEC adapter — check HDMI/USB and power" with retry button (hx-post /ui/action/deep_scan).
devices.html        DevicesPanelData — responsive card grid; each card: display name (fallback chain already computed in Rust), role chip, power badge (on/standby/unknown colored), vendor, HDMI port, ghost badge when discovery=="observed", active-source highlight ring, per-card buttons: Power on, Standby, Set source (hx-post /ui/action/power_on etc. with hx-vals addr). Empty state: "No devices discovered yet — scanning…" + deep scan button.
source_panel.html   RemoteData.hdmi_ports — port strip buttons 1..N (selected state), hx-post /ui/action/hdmi.
volume_panel.html   RemoteData audio fields — big vol-/vol+ buttons (data-repeat attr for long-press), mute toggle with state, volume level bar (audio_display_volume 0..100), unavailable state when !audio_available.
nav_panel.html      RemoteData.nav_targets — target select + D-pad (up/down/left/right/select), back/home/menu row, transport row (play/pause/stop/rew/ff/record). All hx-post /ui/action/nav_key with key names matching registry actions (nav_up, nav_down, nav_left, nav_right, select, back, home, menu, play, pause, stop, fast_forward, rewind, record, channel_up, channel_down, number_0..9).
mqtt_panel.html     MqttPanelData — broker/user/prefix inputs, password (placeholder ••• when pass_set), status dot connected/disconnected, save button hx-post /ui/action/mqtt_save.
health.html         HealthData — version, uptime, lib info, hub subscribers, dropped events, frames captured.
logs.html           LogsData — scrollable mono list, level-colored.
topology_hdmi.html  TopologyData — port rows with device name lists.
device_power.html   single device power cell fragment (DeviceRow).
settings.html       extends shell; cards: Session (mode toggle monitor/passive -> hx-post /ui/dev/action/mode, Reconnect button hx-post /ui/dev/action/mode with mode=reconnect), MQTT (include mqtt_panel), Update (current version, Check button -> GET /api/update/check? NO — use POST /api/update with confirm; show result toast), Security (token configured? show status + link to docs; never display the token).
dev.html            extends shell; dev banner, probe form + results, strategy bench form + results, raw opcode form, frame trace (live table via /ui/dev/fragment/trace polling hx-trigger="every 2s"), event trace feed.
dev_banner.html     DevModeData + adapter state pills.
dev_devices.html    DevicesPanelData reuse for dev page.
dev_trace.html      DevTraceData — frame table rows.
dev_action_result.html  DevActionResult — ok/fail header, strategy table, raw JSON <details>.
event_feed_line.html    EventFeedEntry — one feed line (time, kind chip, summary).
error.html          minimal error page (message).

## Interaction requirements (app.js)

1. SSE: connect EventSource('/api/events'); on devices_changed -> hx.trigger(document.body, 'refresh-devices') style: dispatch htmx trigger `refresh` on #devices-panel and #bus-banner; on source_activated -> refresh source panel; on power_change -> refresh devices; on adapter_state -> refresh banner + toast on disconnect; feed every event into #activity-feed (prepend line rendered client-side from JSON: time, type chip, short summary — build summary in JS per type).
2. Toasts: htmx:responseError -> error toast with status; custom capi:toast event with {level, message}; auto-dismiss 4s, click to dismiss.
3. Long-press repeat: elements with [data-repeat] fire their htmx request on click and then repeat every 350ms while held (min 2 repeats), stop on pointerup/leave.
4. Keyboard shortcuts (only when no input focused): arrows -> nav_key, Enter -> select, Esc/Backspace -> back, +/- volume, m mute, h home, 0-9 numbers. Visual cheat-sheet in dev page footer.
5. Theme toggle: data-theme attr on <html> (dark default), persisted localStorage key capi-theme; respects prefers-color-scheme when unset.
6. Busy states: on htmx:beforeRequest add .busy to the requesting element; remove on afterRequest. Disable double-submit via [data-busy-disable].
7. WS fallback: if EventSource fails twice, open WebSocket /api/events/ws and apply OOB fragments (htmx handles swap when message contains hx-swap-oob; call htmx.process on inserted feed lines).
8. Login page: on 401 responses from any htmx request -> redirect to /login.

## CSS (static/style.css)

Design tokens in :root (dark) + [data-theme="light"] overrides: --bg, --surface, --surface-2, --border, --text, --text-dim, --accent (indigo family), --ok, --warn, --err, --radius, --space-1..6, font stack system-ui. Components: .pill, .card, .grid-devices, .btn (+variants primary/ghost/danger/icon), .dpad, .port-strip, .level-bar, .feed, .table, .toast-stack, .badge-*, .skeleton shimmer, focus-visible rings, prefers-reduced-motion respect, responsive breakpoints (<720px: single column, larger touch targets 48px).

## Acceptance
- Every template compiles conceptually against ui_ctx.rs field names; list any additional precomputed fields you need.
- All hx-* targets exist in the route list above.
- No inline JS beyond data-* attributes; all behavior in app.js.
- style.css complete for every component used by templates.
