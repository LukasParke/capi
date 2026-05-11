package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/LukasParke/capi/cec"
)

// Dev-only HTTP handlers (and a small JSON-API surface that backs the dev
// UI). Anything under /api/dev/* is documented as unstable and intended for
// iterating on CEC behavior, not for external integrations.

// modeRequest is the body for POST /api/dev/mode.
type modeRequest struct {
	// Mode is "passive" (default; we hold a logical address but don't claim
	// active source) or "monitor_only" (libcec doesn't allocate an LA, we
	// can only listen).
	Mode string `json:"mode"`
}

// postDevModeHandler updates the persisted CEC mode in config.json and
// signals the supervisor to reconnect so the new mode takes effect.
//
// POST /api/dev/mode {"mode": "passive" | "monitor_only"}
func postDevModeHandler(w http.ResponseWriter, r *http.Request) {
	var req modeRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "invalid JSON body")
		return
	}
	mode := strings.ToLower(strings.TrimSpace(req.Mode))
	var monitorOnly bool
	switch mode {
	case "passive", "":
		monitorOnly = false
	case "monitor_only", "monitor", "monitoronly":
		monitorOnly = true
	default:
		respondError(w, http.StatusBadRequest, fmt.Sprintf("unknown mode %q (expected passive | monitor_only)", req.Mode))
		return
	}

	configMu.Lock()
	prev := currentConfig.CEC.MonitorOnly
	currentConfig.CEC.MonitorOnly = monitorOnly
	cfg := currentConfig
	path := configFilePath
	configMu.Unlock()

	if err := saveConfig(path, cfg); err != nil {
		respondError(w, http.StatusInternalServerError, fmt.Sprintf("save config: %v", err))
		return
	}

	if prev != monitorOnly {
		appLog("dev", "CEC mode changed to %q; signalling supervisor reconnect", mode)
		signalCECReconnect()
	}

	current := "passive"
	if monitorOnly {
		current = "monitor_only"
	}
	respondSuccess(w, "CEC mode updated", map[string]interface{}{
		"mode":      current,
		"reconnect": prev != monitorOnly,
	})
}

// getDevModeHandler returns the current CEC mode. GET /api/dev/mode.
func getDevModeHandler(w http.ResponseWriter, r *http.Request) {
	configMu.RLock()
	monitorOnly := currentConfig.CEC.MonitorOnly
	activate := currentConfig.CEC.ActivateSource
	configMu.RUnlock()

	mode := "passive"
	if monitorOnly {
		mode = "monitor_only"
	}

	live := "(no adapter)"
	if c := adapter.Conn(); c != nil {
		if c.IsMonitorOnly() {
			live = "monitor_only"
		} else {
			live = "passive"
		}
	}

	respondSuccess(w, "CEC mode", map[string]interface{}{
		"mode":              mode,
		"live_mode":         live,
		"activate_source":   activate,
		"adapter_connected": adapterReady(),
	})
}

// ── Probe ────────────────────────────────────────────────────────────────

// probeRequest is the body of POST /api/dev/probe.
type probeRequest struct {
	Address   int    `json:"address"`
	Kind      string `json:"kind"`                  // power | osd | vendor | cec_version | physical | audio | sam | menu | all
	ObserveMs int    `json:"observe_ms,omitempty"`  // per-probe observation window (default 600ms)
}

// probeStep is one probe + its observed reply frames. Returned per-step so
// the dev UI can attribute each frame to the request that elicited it
// (otherwise an "all" probe drops 8 sends in rapid succession and the
// observation window can't tell which reply came from which send).
type probeStep struct {
	Name      string                  `json:"name"`
	Opcode    string                  `json:"opcode"`
	Error     string                  `json:"error,omitempty"`
	Replies   []decodedFrame          `json:"replies"`
	Result    string                  `json:"result"` // ok | acked_no_reply | feature_aborted | error
	Aborted   int                     `json:"abort_opcode,omitempty"`
	ElapsedMs int64                   `json:"elapsed_ms"`
}

// decodedFrame is BusFrameEntry plus a decoded opcode name and a tiny
// human-readable rendering of common payloads.
type decodedFrame struct {
	BusFrameEntry
	OpcodeName string `json:"opcode_name,omitempty"`
	Decoded    string `json:"decoded,omitempty"`
}

type probeDef struct {
	name   string
	opcode cec.Opcode      // logical opcode being sent (for the response table)
	expect cec.Opcode      // opcode we expect back as a reply (0 = none)
	send   func(*cec.Connection, cec.LogicalAddress) error
}

func builtinProbes() []probeDef {
	return []probeDef{
		{name: "give_power", opcode: cec.OpcodeGiveDevicePowerStatus, expect: cec.OpcodeReportPowerStatus, send: func(c *cec.Connection, la cec.LogicalAddress) error { return c.GiveDevicePowerStatus(la) }},
		{name: "give_vendor", opcode: cec.OpcodeGiveDeviceVendorID, expect: cec.OpcodeDeviceVendorID, send: func(c *cec.Connection, la cec.LogicalAddress) error { return c.GiveDeviceVendorID(la) }},
		{name: "give_osd", opcode: cec.OpcodeGiveOSDName, expect: cec.OpcodeSetOSDName, send: func(c *cec.Connection, la cec.LogicalAddress) error { return c.GiveOSDName(la) }},
		{name: "get_cec_version", opcode: cec.OpcodeGetCECVersion, expect: cec.OpcodeCECVersion, send: func(c *cec.Connection, la cec.LogicalAddress) error {
			return c.Transmit(&cec.Command{Initiator: adapterOwnAddress(c), Destination: la, Opcode: cec.OpcodeGetCECVersion, OpcodeSet: true})
		}},
		{name: "give_physical", opcode: cec.OpcodeGivePhysicalAddress, expect: cec.OpcodeReportPhysicalAddress, send: func(c *cec.Connection, la cec.LogicalAddress) error {
			return c.Transmit(&cec.Command{Initiator: adapterOwnAddress(c), Destination: la, Opcode: cec.OpcodeGivePhysicalAddress, OpcodeSet: true})
		}},
		{name: "give_audio", opcode: cec.OpcodeGiveAudioStatus, expect: cec.OpcodeReportAudioStatus, send: func(c *cec.Connection, la cec.LogicalAddress) error { return c.GiveAudioStatus(la) }},
		{name: "give_sam_status", opcode: cec.OpcodeGiveSystemAudioModeStatus, expect: cec.OpcodeSystemAudioModeStatus, send: func(c *cec.Connection, la cec.LogicalAddress) error { return c.GiveSystemAudioModeStatus(la) }},
		{name: "give_menu_lang", opcode: cec.OpcodeGetMenuLanguage, expect: cec.OpcodeSetMenuLanguage, send: func(c *cec.Connection, la cec.LogicalAddress) error { return c.GiveMenuLanguage(la) }},
	}
}

// postDevProbeHandler runs one (or all) Give* probes against a target LA.
// Probes are sent SEQUENTIALLY with a per-probe observation window so that
// each captured reply can be attributed to the specific probe that
// triggered it. Observe-window default is 600ms per probe (slow LG/Sony
// displays often take 300-500ms to respond); tunable via {"observe_ms": N}.
func postDevProbeHandler(w http.ResponseWriter, r *http.Request) {
	var req probeRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "invalid JSON body")
		return
	}
	if req.Address < 0 || req.Address > 14 {
		respondError(w, http.StatusBadRequest, "address must be 0..14")
		return
	}
	conn := adapter.Conn()
	if conn == nil {
		respondError(w, http.StatusServiceUnavailable, "CEC adapter not available")
		return
	}
	if conn.IsMonitorOnly() {
		respondError(w, http.StatusConflict, "adapter is in monitor-only mode; switch to passive first")
		return
	}
	la := cec.LogicalAddress(req.Address)

	observeMs := req.ObserveMs
	if observeMs <= 0 {
		observeMs = 600
	}

	kind := strings.ToLower(strings.TrimSpace(req.Kind))
	if kind == "" {
		kind = "all"
	}

	all := builtinProbes()
	wanted := make([]probeDef, 0, len(all))
	switch kind {
	case "all":
		wanted = all
	default:
		for _, p := range all {
			if probeMatchesKind(kind, p.name) {
				wanted = append(wanted, p)
			}
		}
		if len(wanted) == 0 {
			respondError(w, http.StatusBadRequest, fmt.Sprintf("unknown probe kind %q", req.Kind))
			return
		}
	}

	steps := make([]probeStep, 0, len(wanted))
	totalReplies := 0
	for _, p := range wanted {
		start := time.Now()
		pre := globalBusState.copyRecentFrames()
		err := p.send(conn, la)

		ps := probeStep{
			Name:    p.name,
			Opcode:  fmt.Sprintf("0x%02X", p.opcode),
			Replies: []decodedFrame{},
		}
		if err != nil {
			ps.Error = err.Error()
			ps.Result = "error"
			ps.ElapsedMs = time.Since(start).Milliseconds()
			steps = append(steps, ps)
			continue
		}
		// Observe.
		time.Sleep(time.Duration(observeMs) * time.Millisecond)
		post := globalBusState.copyRecentFrames()
		new := diffFrames(pre, post)

		ownLA := -1
		if addrs := conn.GetLogicalAddresses(); len(addrs) > 0 {
			ownLA = int(addrs[0])
		}
		for _, f := range new {
			// Skip our own outbound copy that the ring captures too.
			if ownLA >= 0 && f.Initiator == ownLA {
				continue
			}
			df := decodedFrame{BusFrameEntry: f, OpcodeName: opcodeName(opcodeFromHex(f.Opcode))}
			df.Decoded = decodeFramePayload(opcodeFromHex(f.Opcode), f.ParamsHex)
			ps.Replies = append(ps.Replies, df)

			// Classify against the expected reply opcode.
			op := opcodeFromHex(f.Opcode)
			if ps.Result == "" {
				switch {
				case op == cec.OpcodeFeatureAbort && len(f.ParamsHex) >= 1:
					ps.Result = "feature_aborted"
					ps.Aborted = parseHexByte(f.ParamsHex[0])
				case p.expect != 0 && op == p.expect:
					ps.Result = "ok"
				}
			}
		}
		if ps.Result == "" {
			ps.Result = "acked_no_reply"
		}
		ps.ElapsedMs = time.Since(start).Milliseconds()
		totalReplies += len(ps.Replies)
		steps = append(steps, ps)
	}

	body := map[string]interface{}{
		"address":         req.Address,
		"kind":            kind,
		"observe_ms":      observeMs,
		"steps":           steps,
		"total_replies":   totalReplies,
	}
	if cap := globalBusState.frameRingCapacity(); cap == 0 {
		body["note"] = "frame ring is disabled (bus.frame_ring_size < 0); reply frames cannot be captured."
	}
	respondSuccess(w, "probe complete", body)
}

// probeMatchesKind matches "power" -> "give_power" etc., so the dev UI's
// short kind names line up with internal probe names.
func probeMatchesKind(kind, probeName string) bool {
	if kind == probeName {
		return true
	}
	switch kind {
	case "power":
		return probeName == "give_power"
	case "vendor":
		return probeName == "give_vendor"
	case "osd":
		return probeName == "give_osd"
	case "cec_version":
		return probeName == "get_cec_version"
	case "physical":
		return probeName == "give_physical"
	case "audio":
		return probeName == "give_audio"
	case "sam":
		return probeName == "give_sam_status"
	case "menu":
		return probeName == "give_menu_lang"
	}
	return false
}

// decodeFramePayload renders a tiny human-readable interpretation of common
// CEC reply payloads. Returns "" when the opcode/params don't have a known
// decoding (callers fall back to the raw hex).
func decodeFramePayload(op cec.Opcode, paramsHex []string) string {
	switch op {
	case cec.OpcodeReportPowerStatus:
		if len(paramsHex) >= 1 {
			switch parseHexByte(paramsHex[0]) {
			case 0x00:
				return "On"
			case 0x01:
				return "Standby"
			case 0x02:
				return "Transitioning to On"
			case 0x03:
				return "Transitioning to Standby"
			}
		}
	case cec.OpcodeReportAudioStatus:
		if len(paramsHex) >= 1 {
			b := parseHexByte(paramsHex[0])
			muted := (b & 0x80) != 0
			return fmt.Sprintf("vol=%d muted=%v", b&0x7F, muted)
		}
	case cec.OpcodeDeviceVendorID:
		if len(paramsHex) >= 3 {
			vid := uint64(parseHexByte(paramsHex[0]))<<16 | uint64(parseHexByte(paramsHex[1]))<<8 | uint64(parseHexByte(paramsHex[2]))
			return fmt.Sprintf("0x%06X (%s)", vid, cec.GetVendorName(vid))
		}
	case cec.OpcodeReportPhysicalAddress:
		if len(paramsHex) >= 3 {
			a := parseHexByte(paramsHex[0])
			b := parseHexByte(paramsHex[1])
			phys := uint16(a)<<8 | uint16(b)
			return fmt.Sprintf("phys=%s type=0x%02X", cec.PhysicalAddressToString(phys), parseHexByte(paramsHex[2]))
		}
	case cec.OpcodeCECVersion:
		if len(paramsHex) >= 1 {
			return fmt.Sprintf("cec_version=0x%02X", parseHexByte(paramsHex[0]))
		}
	case cec.OpcodeSetOSDName:
		// Decode ASCII bytes.
		out := make([]byte, 0, len(paramsHex))
		for _, h := range paramsHex {
			b := byte(parseHexByte(h))
			if b >= 32 && b < 127 {
				out = append(out, b)
			}
		}
		if len(out) > 0 {
			return fmt.Sprintf("osd=%q", string(out))
		}
	case cec.OpcodeFeatureAbort:
		if len(paramsHex) >= 2 {
			ab := cec.Opcode(parseHexByte(paramsHex[0]))
			reason := parseHexByte(paramsHex[1])
			return fmt.Sprintf("aborted opcode 0x%02X (%s) reason=%d", ab, opcodeName(ab), reason)
		}
	}
	return ""
}

// ── Send key (blind-capable) ─────────────────────────────────────────────

// sendKeyRequest is the body of POST /api/dev/send_key.
type sendKeyRequest struct {
	Address int    `json:"address"`           // 0..14 (raw libcec; we never refuse for "not active")
	Key     string `json:"key,omitempty"`     // canonical key name (preferred)
	Keycode int    `json:"keycode,omitempty"` // 0..255 (used when Key is empty)
	HoldMs  int    `json:"hold_ms,omitempty"` // pause between press and release (0 = no release)
	Wait    bool   `json:"wait,omitempty"`    // libcec wait-for-ack on press/release
	Repeat  int    `json:"repeat,omitempty"`  // press the key N times back-to-back
}

// postDevSendKeyHandler is the dev-UI low-level key sender. Always blind:
// libcec is asked to transmit even if the destination isn't in the active
// mask, so we can probe behind a non-bridging display.
func postDevSendKeyHandler(w http.ResponseWriter, r *http.Request) {
	var req sendKeyRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "invalid JSON body")
		return
	}
	if req.Address < 0 || req.Address > 14 {
		respondError(w, http.StatusBadRequest, "address must be 0..14")
		return
	}
	conn := adapter.Conn()
	if conn == nil {
		respondError(w, http.StatusServiceUnavailable, "CEC adapter not available")
		return
	}
	if conn.IsMonitorOnly() {
		respondError(w, http.StatusConflict, "adapter is in monitor-only mode")
		return
	}

	var key cec.Keycode
	if name := normalizeKeyName(req.Key); name != "" {
		k, ok := keyNameMap[name]
		if !ok {
			respondError(w, http.StatusBadRequest, fmt.Sprintf("unsupported key name %q", req.Key))
			return
		}
		key = k
	} else {
		if req.Keycode < 0 || req.Keycode > 0xFF {
			respondError(w, http.StatusBadRequest, "keycode must be 0..255")
			return
		}
		key = cec.Keycode(req.Keycode)
	}
	repeat := req.Repeat
	if repeat <= 0 {
		repeat = 1
	}
	if repeat > 32 {
		repeat = 32
	}

	target := cec.LogicalAddress(req.Address)
	pre := globalBusState.copyRecentFrames()
	type sendStep struct {
		Phase string `json:"phase"`
		Acked bool   `json:"acked"`
		Error string `json:"error,omitempty"`
	}
	steps := make([]sendStep, 0, repeat*2)
	for i := 0; i < repeat; i++ {
		err := conn.SendKeypress(target, key, req.Wait)
		ps := sendStep{Phase: "press", Acked: err == nil}
		if err != nil {
			ps.Error = err.Error()
		}
		steps = append(steps, ps)
		if req.HoldMs > 0 {
			time.Sleep(time.Duration(req.HoldMs) * time.Millisecond)
			err := conn.SendKeyRelease(target, req.Wait)
			rs := sendStep{Phase: "release", Acked: err == nil}
			if err != nil {
				rs.Error = err.Error()
			}
			steps = append(steps, rs)
		}
		if i+1 < repeat {
			time.Sleep(100 * time.Millisecond)
		}
	}
	time.Sleep(700 * time.Millisecond)
	post := globalBusState.copyRecentFrames()
	respondSuccess(w, "send_key complete", map[string]interface{}{
		"address":    req.Address,
		"keycode":    int(key),
		"hold_ms":    req.HoldMs,
		"repeat":     repeat,
		"steps":      steps,
		"new_frames": diffFrames(pre, post),
	})
}

// ── Send opcode (raw transmit) ──────────────────────────────────────────

// sendOpcodeRequest is the body of POST /api/dev/send_opcode.
type sendOpcodeRequest struct {
	Destination int    `json:"destination"`
	Opcode      int    `json:"opcode"`
	ParamsHex   string `json:"params_hex,omitempty"` // "01 02 ff" or "0102ff"
}

// postDevSendOpcodeHandler is the dev-UI raw-CEC sender. Always blind.
func postDevSendOpcodeHandler(w http.ResponseWriter, r *http.Request) {
	var req sendOpcodeRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "invalid JSON body")
		return
	}
	if req.Destination < 0 || req.Destination > 15 {
		respondError(w, http.StatusBadRequest, "destination must be 0..15")
		return
	}
	if req.Opcode < 0 || req.Opcode > 0xFF {
		respondError(w, http.StatusBadRequest, "opcode must be 0..255")
		return
	}
	conn := adapter.Conn()
	if conn == nil {
		respondError(w, http.StatusServiceUnavailable, "CEC adapter not available")
		return
	}
	if conn.IsMonitorOnly() {
		respondError(w, http.StatusConflict, "adapter is in monitor-only mode")
		return
	}
	params, err := parseHexBytes(req.ParamsHex)
	if err != nil {
		respondError(w, http.StatusBadRequest, fmt.Sprintf("invalid params_hex: %v", err))
		return
	}
	if len(params) > 14 {
		respondError(w, http.StatusBadRequest, "too many parameters (max 14)")
		return
	}
	pre := globalBusState.copyRecentFrames()
	cmd := &cec.Command{
		Initiator:   adapterOwnAddress(conn),
		Destination: cec.LogicalAddress(req.Destination),
		Opcode:      cec.Opcode(req.Opcode),
		OpcodeSet:   true,
		Parameters:  params,
	}
	txErr := conn.Transmit(cmd)
	time.Sleep(700 * time.Millisecond)
	post := globalBusState.copyRecentFrames()
	out := map[string]interface{}{
		"destination": req.Destination,
		"opcode":      req.Opcode,
		"params_hex":  hex.EncodeToString(params),
		"acked":       txErr == nil,
		"new_frames":  diffFrames(pre, post),
	}
	if txErr != nil {
		out["error"] = txErr.Error()
	}
	respondSuccess(w, "send_opcode complete", out)
}

func parseHexBytes(s string) ([]byte, error) {
	s = strings.TrimSpace(s)
	if s == "" {
		return nil, nil
	}
	// Strip separators.
	cleaned := make([]byte, 0, len(s))
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c == ' ' || c == ',' || c == ':' {
			continue
		}
		cleaned = append(cleaned, c)
	}
	if len(cleaned)%2 != 0 {
		return nil, fmt.Errorf("hex string has odd length")
	}
	return hex.DecodeString(string(cleaned))
}

// ── Strategy bench ───────────────────────────────────────────────────────

// runStrategiesRequest is the body of POST /api/dev/run_strategies.
type runStrategiesRequest struct {
	Action      string `json:"action"`
	Target      *int   `json:"target,omitempty"` // null = follow the strategy's own targets
	AllStrategies bool `json:"all_strategies,omitempty"`
	ObserveMs   int    `json:"observe_ms,omitempty"`
}

// postDevRunStrategiesHandler runs the strategy chain for an action and
// returns the per-strategy result table. When all_strategies = true (the
// dev UI's "Run all" button), every strategy in the chain is executed even
// after one succeeds, so the user can compare outcomes.
func postDevRunStrategiesHandler(w http.ResponseWriter, r *http.Request) {
	var req runStrategiesRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "invalid JSON body")
		return
	}
	action, ok := ParseAction(req.Action)
	if !ok {
		respondError(w, http.StatusBadRequest, fmt.Sprintf("unknown action %q", req.Action))
		return
	}
	conn := adapter.Conn()
	if conn == nil {
		respondError(w, http.StatusServiceUnavailable, "CEC adapter not available")
		return
	}
	if conn.IsMonitorOnly() {
		respondError(w, http.StatusConflict, "adapter is in monitor-only mode")
		return
	}
	target := cec.LogicalAddressUnknown
	if req.Target != nil {
		if *req.Target < 0 || *req.Target > 14 {
			respondError(w, http.StatusBadRequest, "target must be 0..14")
			return
		}
		target = cec.LogicalAddress(*req.Target)
	}
	vendor := vendorIDForTarget(target)
	ctx, cancel := context.WithTimeout(r.Context(), 30*time.Second)
	defer cancel()

	results, err := defaultRegistry.Run(ctx, conn, action, RunOptions{
		Vendor:            vendor,
		Target:            target,
		AllStrategies:     req.AllStrategies,
		ObserveOverrideMs: req.ObserveMs,
	})
	if err != nil {
		respondError(w, http.StatusInternalServerError, err.Error())
		return
	}
	respondSuccess(w, "strategies run", map[string]interface{}{
		"action":  action.String(),
		"target":  int(target),
		"vendor":  vendor,
		"results": results,
	})
}

// ── Save strategy winner ────────────────────────────────────────────────

// saveStrategyRequest is the body of POST /api/dev/save_strategy.
type saveStrategyRequest struct {
	Vendor   string `json:"vendor"`   // "0x000048"
	Action   string `json:"action"`   // "volume_up"
	Strategy string `json:"strategy"` // strategy name from the bench result
}

// postDevSaveStrategyHandler installs a winning strategy as the per-vendor
// default for an action. Persisted into config.json under "cec.strategies".
//
// The strategy referenced by name must exist in the default chain for that
// action; we don't yet support custom user-defined strategies (that's a
// follow-up if it's needed).
func postDevSaveStrategyHandler(w http.ResponseWriter, r *http.Request) {
	var req saveStrategyRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		respondError(w, http.StatusBadRequest, "invalid JSON body")
		return
	}
	action, ok := ParseAction(req.Action)
	if !ok {
		respondError(w, http.StatusBadRequest, fmt.Sprintf("unknown action %q", req.Action))
		return
	}
	vendor := strings.ToLower(strings.TrimSpace(req.Vendor))
	if vendor == "" {
		respondError(w, http.StatusBadRequest, "vendor required")
		return
	}
	name := strings.TrimSpace(req.Strategy)
	if name == "" {
		respondError(w, http.StatusBadRequest, "strategy required")
		return
	}

	// Resolve the named strategy from the action's default chain.
	var picked *Strategy
	for _, s := range defaultRegistry.StrategiesFor("", action) {
		if s.Name == name {
			s := s
			picked = &s
			break
		}
	}
	if picked == nil {
		respondError(w, http.StatusBadRequest, fmt.Sprintf("strategy %q not found in default chain for %s", name, action))
		return
	}

	defaultRegistry.SetVendorOverride(vendor, action, []Strategy{*picked})

	// Persist to config.
	configMu.Lock()
	if currentConfig.CEC.StrategyOverrides == nil {
		currentConfig.CEC.StrategyOverrides = map[string]map[string]string{}
	}
	if currentConfig.CEC.StrategyOverrides[vendor] == nil {
		currentConfig.CEC.StrategyOverrides[vendor] = map[string]string{}
	}
	currentConfig.CEC.StrategyOverrides[vendor][action.String()] = name
	cfg := currentConfig
	path := configFilePath
	configMu.Unlock()
	if err := saveConfig(path, cfg); err != nil {
		respondError(w, http.StatusInternalServerError, fmt.Sprintf("save config: %v", err))
		return
	}
	respondSuccess(w, "strategy saved", map[string]interface{}{
		"vendor":   vendor,
		"action":   action.String(),
		"strategy": name,
	})
}

// ── Misc ─────────────────────────────────────────────────────────────────

// getDevActionsHandler returns the list of registry actions, useful for the
// strategy-bench dropdown.
func getDevActionsHandler(w http.ResponseWriter, r *http.Request) {
	out := []string{}
	for a := Action(1); a <= ActNumber9; a++ {
		out = append(out, a.String())
	}
	respondSuccess(w, "actions", out)
}

// getDevKeysHandler returns the canonical key-name list for the keycode
// dropdown.
func getDevKeysHandler(w http.ResponseWriter, r *http.Request) {
	out := make([]map[string]interface{}, 0, len(keyNameMap))
	for name, code := range keyNameMap {
		out = append(out, map[string]interface{}{
			"name":    name,
			"keycode": int(code),
		})
	}
	respondSuccess(w, "keys", out)
}

// getDevOpcodesHandler returns named CEC opcodes for the raw-opcode picker.
func getDevOpcodesHandler(w http.ResponseWriter, r *http.Request) {
	out := make([]map[string]interface{}, 0, len(opcodeNames))
	for _, n := range opcodeNames {
		out = append(out, map[string]interface{}{
			"name":   n.name,
			"opcode": int(n.op),
			"hex":    fmt.Sprintf("0x%02X", n.op),
		})
	}
	respondSuccess(w, "opcodes", out)
}

// devModeAvailable returns the small subset of dev-UI metadata templates
// need (mode + adapter status + counts) without re-fetching JSON.
func devUIBanner() map[string]interface{} {
	configMu.RLock()
	monitorOnly := currentConfig.CEC.MonitorOnly
	configMu.RUnlock()
	mode := "passive"
	if monitorOnly {
		mode = "monitor_only"
	}
	live := "(no adapter)"
	conn := adapter.Conn()
	if conn != nil {
		if conn.IsMonitorOnly() {
			live = "monitor_only"
		} else {
			live = "passive"
		}
	}
	snap := globalBusState.copySnapshot()
	return map[string]interface{}{
		"Mode":        mode,
		"LiveMode":    live,
		"Adapter":     live != "(no adapter)",
		"Devices":     len(snap.Devices),
		"FrameRing":   snap.FrameRingSize,
		"FrameCount":  len(snap.RecentFrames),
		"ActiveSrc":   snap.ActiveSource,
		"GeneratedAt": time.Now().Format(time.RFC3339),
	}
}

// formInt is a tiny helper for the HTMX action endpoints below.
func formInt(r *http.Request, key string) (int, bool) {
	s := strings.TrimSpace(r.FormValue(key))
	if s == "" {
		return 0, false
	}
	v, err := strconv.Atoi(s)
	if err != nil {
		return 0, false
	}
	return v, true
}

// ── HTMX layout + fragments ──────────────────────────────────────────────

// devLayoutHandler serves the /dev page, the developer-focused control +
// observation surface. Pulls the same key/opcode/action lists as the JSON
// endpoints so the picker dropdowns stay in sync.
func devLayoutHandler(w http.ResponseWriter, r *http.Request) {
	keyNames := make([]string, 0, len(keyNameMap))
	for name := range keyNameMap {
		keyNames = append(keyNames, name)
	}
	sortedStrings(keyNames)

	type opcodeRow struct {
		Name   string
		Hex    string
		Opcode int
	}
	opRows := make([]opcodeRow, 0, len(opcodeNames))
	for _, n := range opcodeNames {
		opRows = append(opRows, opcodeRow{
			Name:   n.name,
			Hex:    fmt.Sprintf("0x%02X", n.op),
			Opcode: int(n.op),
		})
	}

	actionNames := make([]string, 0, 32)
	for a := Action(1); a <= ActNumber9; a++ {
		actionNames = append(actionNames, a.String())
	}

	writeHTMLFragment(w, "dev_layout", map[string]interface{}{
		"Version":  version,
		"KeyNames": keyNames,
		"Opcodes":  opRows,
		"Actions":  actionNames,
	})
}

// uiDevFragmentBanner renders the top status line.
func uiDevFragmentBanner(w http.ResponseWriter, r *http.Request) {
	writeHTMLFragment(w, "dev_banner", devUIBanner())
}

// uiDevFragmentDevices renders the deep device cards (using the same row
// builder as the / dashboard, but with the ghost devices and probe buttons).
func uiDevFragmentDevices(w http.ResponseWriter, r *http.Request) {
	snap := globalBusState.copySnapshot()
	own := topologyOwnAddresses()
	rows := buildDeviceRowsFromMaps(snap.Devices, own, snap.ActiveSource)
	writeHTMLFragment(w, "dev_devices", map[string]interface{}{"Devices": rows})
}

// uiDevFragmentTrace renders the live frame ring with light filtering.
func uiDevFragmentTrace(w http.ResponseWriter, r *http.Request) {
	snap := globalBusState.copySnapshot()
	frames := snap.RecentFrames
	// Keep the most recent 50 by default to keep the render light.
	maxRows := 50
	if v, ok := formInt(r, "max"); ok && v > 0 {
		maxRows = v
	}
	if len(frames) > maxRows {
		frames = frames[len(frames)-maxRows:]
	}
	type frameRow struct {
		Timestamp   string
		Initiator   int
		Destination int
		Opcode      string
		ParamsHex   []string
	}
	rendered := make([]frameRow, 0, len(frames))
	for _, f := range frames {
		rendered = append(rendered, frameRow{
			Timestamp:   f.Timestamp.Format("15:04:05.000"),
			Initiator:   f.Initiator,
			Destination: f.Destination,
			Opcode:      f.Opcode,
			ParamsHex:   f.ParamsHex,
		})
	}
	writeHTMLFragment(w, "dev_trace", map[string]interface{}{
		"Frames": rendered,
		"Count":  len(snap.RecentFrames),
		"Cap":    snap.FrameRingSize,
	})
}

// devActionResult writes a small HTMX-friendly result panel summarising a
// dev action's JSON outcome. Used by every /ui/dev/action/* handler.
func devActionResult(w http.ResponseWriter, ok bool, title string, body interface{}) {
	cls := "result ok"
	if !ok {
		cls = "result err"
	}
	pretty, _ := json.MarshalIndent(body, "", "  ")
	fmt.Fprintf(w, `<div class="%s"><div><strong>%s</strong></div><pre style="white-space:pre-wrap">%s</pre></div>`,
		cls, htmlEscape(title), htmlEscape(string(pretty)))
}

func htmlEscape(s string) string {
	r := strings.NewReplacer("&", "&amp;", "<", "&lt;", ">", "&gt;", "\"", "&quot;")
	return r.Replace(s)
}

// uiDevActionMode is the HTMX form-encoded equivalent of POST /api/dev/mode.
func uiDevActionMode(w http.ResponseWriter, r *http.Request) {
	mode := strings.TrimSpace(r.FormValue("mode"))
	body, _ := json.Marshal(modeRequest{Mode: mode})
	r2 := newSubrequest(r, body)
	postDevModeHandlerWrapped(w, r2)
}

// uiDevActionProbe wraps POST /api/dev/probe for HTMX form params.
func uiDevActionProbe(w http.ResponseWriter, r *http.Request) {
	addr, _ := formInt(r, "addr")
	kind := strings.TrimSpace(r.URL.Query().Get("kind"))
	if kind == "" {
		kind = "all"
	}
	body, _ := json.Marshal(probeRequest{Address: addr, Kind: kind})
	postDevProbeHandlerWrapped(w, newSubrequest(r, body))
}

// uiDevActionSendKey wraps POST /api/dev/send_key for HTMX form params.
func uiDevActionSendKey(w http.ResponseWriter, r *http.Request) {
	if err := r.ParseForm(); err != nil {
		respondError(w, http.StatusBadRequest, err.Error())
		return
	}
	addr, _ := formInt(r, "addr")
	hold, _ := formInt(r, "hold_ms")
	repeat, _ := formInt(r, "repeat")
	body, _ := json.Marshal(sendKeyRequest{
		Address: addr,
		Key:     r.FormValue("key"),
		HoldMs:  hold,
		Wait:    r.FormValue("wait") != "",
		Repeat:  repeat,
	})
	postDevSendKeyHandlerWrapped(w, newSubrequest(r, body))
}

// uiDevActionSendOpcode wraps POST /api/dev/send_opcode for HTMX form params.
func uiDevActionSendOpcode(w http.ResponseWriter, r *http.Request) {
	if err := r.ParseForm(); err != nil {
		respondError(w, http.StatusBadRequest, err.Error())
		return
	}
	dest, _ := formInt(r, "dest")
	op, _ := formInt(r, "opcode")
	body, _ := json.Marshal(sendOpcodeRequest{
		Destination: dest,
		Opcode:      op,
		ParamsHex:   r.FormValue("params_hex"),
	})
	postDevSendOpcodeHandlerWrapped(w, newSubrequest(r, body))
}

// uiDevActionRunStrategies wraps POST /api/dev/run_strategies and renders
// the per-strategy result table via the dev_strategy_results template.
func uiDevActionRunStrategies(w http.ResponseWriter, r *http.Request) {
	if err := r.ParseForm(); err != nil {
		respondError(w, http.StatusBadRequest, err.Error())
		return
	}
	action, ok := ParseAction(r.FormValue("action"))
	if !ok {
		writeHTMLFragment(w, "action_note", map[string]interface{}{
			"OK": false, "Text": fmt.Sprintf("unknown action %q", r.FormValue("action")),
		})
		return
	}
	conn := adapter.Conn()
	if conn == nil {
		writeHTMLFragment(w, "action_note", map[string]interface{}{
			"OK": false, "Text": "CEC adapter not available",
		})
		return
	}
	if conn.IsMonitorOnly() {
		writeHTMLFragment(w, "action_note", map[string]interface{}{
			"OK": false, "Text": "adapter is in monitor-only mode",
		})
		return
	}
	target := cec.LogicalAddressUnknown
	if v, present := formInt(r, "target"); present {
		if v < 0 || v > 14 {
			writeHTMLFragment(w, "action_note", map[string]interface{}{
				"OK": false, "Text": "target must be 0..14",
			})
			return
		}
		target = cec.LogicalAddress(v)
	}
	observe, _ := formInt(r, "observe_ms")
	all := r.FormValue("all_strategies") != ""
	vendor := vendorIDForTarget(target)

	ctx, cancel := context.WithTimeout(r.Context(), 30*time.Second)
	defer cancel()
	results, err := defaultRegistry.Run(ctx, conn, action, RunOptions{
		Vendor:            vendor,
		Target:            target,
		AllStrategies:     all,
		ObserveOverrideMs: observe,
	})
	if err != nil {
		writeHTMLFragment(w, "action_note", map[string]interface{}{
			"OK": false, "Text": err.Error(),
		})
		return
	}
	writeHTMLFragment(w, "dev_strategy_results", map[string]interface{}{
		"Action":  action.String(),
		"Vendor":  vendor,
		"Results": results,
	})
}

// uiDevActionSaveStrategy wraps POST /api/dev/save_strategy for HTMX query params.
func uiDevActionSaveStrategy(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	body, _ := json.Marshal(saveStrategyRequest{
		Vendor:   q.Get("vendor"),
		Action:   q.Get("action"),
		Strategy: q.Get("strategy"),
	})
	postDevSaveStrategyHandlerWrapped(w, newSubrequest(r, body))
}

// ── Subrequest plumbing ──────────────────────────────────────────────────
//
// The /ui/dev/action/* endpoints reuse the JSON handlers above via tiny
// "wrapped" shims. The shim replaces the response writer with one that
// renders the JSON envelope as a styled HTMX panel instead of a raw JSON
// stream, so the dev UI can drop the result into #dev-result.

type devActionWriter struct {
	http.ResponseWriter
	buf  []byte
	code int
}

func (w *devActionWriter) Write(p []byte) (int, error) {
	w.buf = append(w.buf, p...)
	return len(p), nil
}
func (w *devActionWriter) WriteHeader(c int) { w.code = c }

func newSubrequest(orig *http.Request, body []byte) *http.Request {
	r := orig.Clone(orig.Context())
	r.Body = noopBodyReader(body)
	r.ContentLength = int64(len(body))
	r.Header.Set("Content-Type", "application/json")
	return r
}

type noopBody struct {
	data []byte
	pos  int
}

func (b *noopBody) Read(p []byte) (int, error) {
	if b.pos >= len(b.data) {
		return 0, errEOF
	}
	n := copy(p, b.data[b.pos:])
	b.pos += n
	return n, nil
}
func (b *noopBody) Close() error { return nil }

var errEOF = fmt.Errorf("EOF")

func noopBodyReader(b []byte) *noopBody { return &noopBody{data: b} }

// jsonToHTMX runs an inner JSON handler against a fake writer, then renders
// the captured response as an HTMX result panel.
func jsonToHTMX(inner func(w http.ResponseWriter, r *http.Request), title string) func(http.ResponseWriter, *http.Request) {
	return func(w http.ResponseWriter, r *http.Request) {
		rec := &devActionWriter{ResponseWriter: w}
		inner(rec, r)
		var env Response
		if err := json.Unmarshal(rec.buf, &env); err != nil {
			devActionResult(w, false, title, map[string]string{"raw": string(rec.buf)})
			return
		}
		ok := env.Status == "success" || env.Status == "accepted"
		body := map[string]interface{}{
			"message": env.Message,
			"data":    env.Data,
		}
		devActionResult(w, ok, title, body)
	}
}

func postDevModeHandlerWrapped(w http.ResponseWriter, r *http.Request) {
	jsonToHTMX(postDevModeHandler, "Mode")(w, r)
}
func postDevProbeHandlerWrapped(w http.ResponseWriter, r *http.Request) {
	jsonToHTMX(postDevProbeHandler, "Probe")(w, r)
}
func postDevSendKeyHandlerWrapped(w http.ResponseWriter, r *http.Request) {
	jsonToHTMX(postDevSendKeyHandler, "Send key")(w, r)
}
func postDevSendOpcodeHandlerWrapped(w http.ResponseWriter, r *http.Request) {
	jsonToHTMX(postDevSendOpcodeHandler, "Send opcode")(w, r)
}
func postDevSaveStrategyHandlerWrapped(w http.ResponseWriter, r *http.Request) {
	jsonToHTMX(postDevSaveStrategyHandler, "Save strategy")(w, r)
}

// sortedStrings sorts in place. Tiny helper kept here to avoid importing
// sort across two files.
func sortedStrings(s []string) {
	for i := 1; i < len(s); i++ {
		for j := i; j > 0 && s[j-1] > s[j]; j-- {
			s[j-1], s[j] = s[j], s[j-1]
		}
	}
}
