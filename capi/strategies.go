package main

import (
	"context"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/LukasParke/capi/cec"
)

// Action enumerates the user-level CEC actions the registry knows about.
// Each action maps to one or more Strategies (ordered chains of CEC steps)
// that we try in sequence until one succeeds.
type Action int

const (
	ActUnknown Action = iota
	ActVolumeUp
	ActVolumeDown
	ActMute
	ActNavUp
	ActNavDown
	ActNavLeft
	ActNavRight
	ActSelect
	ActBack
	ActHome
	ActMenu
	ActChannelUp
	ActChannelDown
	ActPlay
	ActPause
	ActStop
	ActFastForward
	ActRewind
	ActRecord
	ActPower
	ActNumber0
	ActNumber1
	ActNumber2
	ActNumber3
	ActNumber4
	ActNumber5
	ActNumber6
	ActNumber7
	ActNumber8
	ActNumber9
)

// String returns the canonical lowercase identifier used in HTTP/JSON paths.
func (a Action) String() string {
	switch a {
	case ActVolumeUp:
		return "volume_up"
	case ActVolumeDown:
		return "volume_down"
	case ActMute:
		return "mute"
	case ActNavUp:
		return "nav_up"
	case ActNavDown:
		return "nav_down"
	case ActNavLeft:
		return "nav_left"
	case ActNavRight:
		return "nav_right"
	case ActSelect:
		return "select"
	case ActBack:
		return "back"
	case ActHome:
		return "home"
	case ActMenu:
		return "menu"
	case ActChannelUp:
		return "channel_up"
	case ActChannelDown:
		return "channel_down"
	case ActPlay:
		return "play"
	case ActPause:
		return "pause"
	case ActStop:
		return "stop"
	case ActFastForward:
		return "fast_forward"
	case ActRewind:
		return "rewind"
	case ActRecord:
		return "record"
	case ActPower:
		return "power"
	case ActNumber0:
		return "number_0"
	case ActNumber1:
		return "number_1"
	case ActNumber2:
		return "number_2"
	case ActNumber3:
		return "number_3"
	case ActNumber4:
		return "number_4"
	case ActNumber5:
		return "number_5"
	case ActNumber6:
		return "number_6"
	case ActNumber7:
		return "number_7"
	case ActNumber8:
		return "number_8"
	case ActNumber9:
		return "number_9"
	default:
		return "unknown"
	}
}

// ParseAction parses the canonical lowercase identifier (case- and
// separator-insensitive: "volume_up" / "volume-up" / "VolumeUp" all work).
func ParseAction(s string) (Action, bool) {
	norm := strings.ToLower(strings.NewReplacer("-", "_", " ", "_").Replace(strings.TrimSpace(s)))
	for a := Action(1); a <= ActNumber9; a++ {
		if a.String() == norm {
			return a, true
		}
	}
	return ActUnknown, false
}

// StepKind enumerates the primitive CEC operations a Strategy can chain.
type StepKind int

const (
	StepNone StepKind = iota

	// StepSendUserControl sends a User Control Pressed + (optional) Released
	// pair using libcec_send_keypress / libcec_send_key_release. HoldMs is
	// the pause between press and release; <= 0 sends only the press.
	StepSendUserControl

	// StepTransmit sends a single raw cec.Command via libcec_transmit. Used
	// for opcodes libcec doesn't have a dedicated wrapper for.
	StepTransmit

	// StepLibcecVolumeUp / StepLibcecVolumeDown / StepLibcecMute call the
	// libcec convenience wrappers, which target whatever libcec considers
	// the audio system.
	StepLibcecVolumeUp
	StepLibcecVolumeDown
	StepLibcecMute

	// StepLibcecPowerOn / StepLibcecStandby call libcec_power_on_devices /
	// libcec_standby_devices on a specific LA.
	StepLibcecPowerOn
	StepLibcecStandby

	// StepEnableSAM sends Set System Audio Mode (0x72) to the TV. Bus
	// state-changing; only used by strategies that explicitly opt in.
	StepEnableSAM

	// StepWait pauses for DelayMs (no I/O). Used to give devices time to
	// react between steps.
	StepWait
)

// Step is one primitive operation in a Strategy chain.
type Step struct {
	Kind   StepKind
	Target cec.LogicalAddress
	Key    cec.Keycode
	Wait   bool // libcec wait-for-ack flag on the press
	HoldMs int  // pause between press and release (0 = no release)
	Opcode cec.Opcode
	Params []byte

	// DelayMs (for StepWait) or post-step settle delay (otherwise).
	DelayMs int
}

// Strategy is an ordered list of Steps tried as a unit. The executor runs
// every step in order; if any step's libcec_transmit returns no-ack the
// strategy is marked acked=false but execution continues so we observe any
// late replies from the bus.
type Strategy struct {
	Name      string
	Steps     []Step
	ObserveMs int // how long to watch the bus for replies after the last step (default 500ms)
}

// StratResultStatus classifies the observed outcome of running a strategy.
type StratResultStatus string

const (
	StratStatusOK             StratResultStatus = "ok"               // got an expected reply
	StratStatusAckedNoReply   StratResultStatus = "acked_no_reply"   // libcec acked, no observed reply
	StratStatusFeatureAborted StratResultStatus = "feature_aborted"  // saw FeatureAbort referencing our opcode
	StratStatusNoAck          StratResultStatus = "no_ack"           // libcec_transmit returned no-ack on at least one step
	StratStatusError          StratResultStatus = "error"            // step returned a hard error
	StratStatusSkipped        StratResultStatus = "skipped"          // skipped (e.g. monitor-only mode, no adapter)
)

// StratResult is the per-strategy report from Registry.Run.
type StratResult struct {
	Strategy   string            `json:"strategy"`
	Status     StratResultStatus `json:"status"`
	Acked      bool              `json:"acked"`
	ReplyOpcode int              `json:"reply_opcode,omitempty"` // 0 = none
	ReplyName  string            `json:"reply_name,omitempty"`
	AbortOpcode int              `json:"abort_opcode,omitempty"`
	ElapsedMs  int64             `json:"elapsed_ms"`
	Error      string            `json:"error,omitempty"`
	Steps      []StepResult      `json:"steps,omitempty"`
}

// StepResult is the per-step record inside a StratResult.
type StepResult struct {
	Kind   string `json:"kind"`
	Target int    `json:"target,omitempty"`
	Acked  bool   `json:"acked"`
	Error  string `json:"error,omitempty"`
}

// expectedReplyOpcode returns the CEC reply opcode that maps to a step's
// request, used by the executor to classify "ok" vs "acked_no_reply".
// Returns 0 when there's no canonical reply.
func expectedReplyOpcode(s Step) cec.Opcode {
	switch s.Kind {
	case StepSendUserControl:
		switch s.Key {
		case cec.KeycodeVolumeUp, cec.KeycodeVolumeDown, cec.KeycodeMute:
			return cec.OpcodeReportAudioStatus
		}
	case StepLibcecVolumeUp, StepLibcecVolumeDown, StepLibcecMute:
		return cec.OpcodeReportAudioStatus
	case StepLibcecPowerOn, StepLibcecStandby:
		return cec.OpcodeReportPowerStatus
	case StepTransmit:
		switch s.Opcode {
		case cec.OpcodeGiveDevicePowerStatus:
			return cec.OpcodeReportPowerStatus
		case cec.OpcodeGiveAudioStatus:
			return cec.OpcodeReportAudioStatus
		case cec.OpcodeGiveDeviceVendorID:
			return cec.OpcodeDeviceVendorID
		case cec.OpcodeGiveOSDName:
			return cec.OpcodeSetOSDName
		case cec.OpcodeGivePhysicalAddress:
			return cec.OpcodeReportPhysicalAddress
		case cec.OpcodeGetCECVersion:
			return cec.OpcodeCECVersion
		}
	}
	return 0
}

// Registry maps Actions to ordered Strategy chains, with optional per-vendor
// overrides (keyed by lowercase 0x-prefixed vendor ID, e.g. "0x000048").
type Registry struct {
	mu        sync.RWMutex
	defaults  map[Action][]Strategy
	perVendor map[string]map[Action][]Strategy
}

// NewRegistry returns a Registry seeded with built-in defaults.
func NewRegistry() *Registry {
	r := &Registry{
		defaults:  defaultStrategies(),
		perVendor: map[string]map[Action][]Strategy{},
	}
	return r
}

// SetVendorOverride installs a list of strategies for an action under a
// specific vendor. nil clears any override.
func (r *Registry) SetVendorOverride(vendor string, action Action, strategies []Strategy) {
	r.mu.Lock()
	defer r.mu.Unlock()
	v := strings.ToLower(strings.TrimSpace(vendor))
	if v == "" {
		return
	}
	if r.perVendor[v] == nil {
		r.perVendor[v] = map[Action][]Strategy{}
	}
	if strategies == nil {
		delete(r.perVendor[v], action)
		if len(r.perVendor[v]) == 0 {
			delete(r.perVendor, v)
		}
		return
	}
	r.perVendor[v][action] = strategies
}

// StrategiesFor returns the ordered strategy chain for an action, preferring
// a per-vendor override when one is registered.
func (r *Registry) StrategiesFor(vendor string, action Action) []Strategy {
	r.mu.RLock()
	defer r.mu.RUnlock()
	v := strings.ToLower(strings.TrimSpace(vendor))
	if v != "" {
		if perAct, ok := r.perVendor[v]; ok {
			if s, ok := perAct[action]; ok && len(s) > 0 {
				return s
			}
		}
	}
	return r.defaults[action]
}

// RunOptions tweaks Run.
type RunOptions struct {
	// Vendor of the destination device (lowercase 0x-prefixed); used to
	// pick per-vendor overrides. Empty = use defaults.
	Vendor string

	// Target overrides the destination LA in any step whose target is
	// LogicalAddressUnknown. Useful for "send to active source" strategies.
	Target cec.LogicalAddress

	// AllStrategies, when true, runs every strategy in the chain instead
	// of stopping at the first ok. Used by the dev UI's strategy bench.
	AllStrategies bool

	// ObserveOverrideMs overrides each strategy's ObserveMs (0 = use the
	// strategy's own value, defaulting to 500ms).
	ObserveOverrideMs int
}

// Run executes the strategy chain for an action, returning a per-strategy
// result table. Stops at the first ok unless RunOptions.AllStrategies.
//
// Run captures the bus frame ring length before each strategy, executes the
// strategy's steps, waits ObserveMs, then inspects the new frames to
// classify the outcome.
func (r *Registry) Run(ctx context.Context, conn *cec.Connection, action Action, opts RunOptions) ([]StratResult, error) {
	if conn == nil {
		return nil, fmt.Errorf("strategies: no live cec connection")
	}
	if conn.IsMonitorOnly() {
		return nil, fmt.Errorf("strategies: connection is monitor-only")
	}
	chain := r.StrategiesFor(opts.Vendor, action)
	if len(chain) == 0 {
		return nil, fmt.Errorf("strategies: no strategies registered for %s", action)
	}

	results := make([]StratResult, 0, len(chain))
	for _, s := range chain {
		if ctx.Err() != nil {
			return results, ctx.Err()
		}
		res := executeStrategy(ctx, conn, s, opts)
		results = append(results, res)
		if !opts.AllStrategies && res.Status == StratStatusOK {
			break
		}
	}
	return results, nil
}

// executeStrategy runs one Strategy end-to-end and classifies the outcome.
func executeStrategy(ctx context.Context, conn *cec.Connection, s Strategy, opts RunOptions) StratResult {
	start := time.Now()
	res := StratResult{
		Strategy: s.Name,
		Acked:    true,
	}

	// Snapshot the frame ring so we can diff after the run.
	preFrames := globalBusState.copyRecentFrames()

	var lastExpectedReply cec.Opcode
	for _, st := range s.Steps {
		stRes := StepResult{Kind: stepKindString(st.Kind), Target: int(st.Target)}
		if st.Target == cec.LogicalAddressUnknown && opts.Target != cec.LogicalAddressUnknown {
			st.Target = opts.Target
			stRes.Target = int(opts.Target)
		}
		if st.Kind == StepWait {
			if st.DelayMs > 0 {
				if !sleepCtx(ctx, time.Duration(st.DelayMs)*time.Millisecond) {
					res.Status = StratStatusError
					res.Error = ctx.Err().Error()
					res.ElapsedMs = time.Since(start).Milliseconds()
					return res
				}
			}
			res.Steps = append(res.Steps, stRes)
			continue
		}

		err := executeStep(conn, st)
		if err != nil {
			stRes.Error = err.Error()
			stRes.Acked = false
			res.Acked = false
			res.Steps = append(res.Steps, stRes)
			// Don't bail; continue collecting evidence.
			continue
		}
		stRes.Acked = true
		if exp := expectedReplyOpcode(st); exp != 0 {
			lastExpectedReply = exp
		}
		res.Steps = append(res.Steps, stRes)

		if st.DelayMs > 0 {
			if !sleepCtx(ctx, time.Duration(st.DelayMs)*time.Millisecond) {
				res.Status = StratStatusError
				res.Error = ctx.Err().Error()
				res.ElapsedMs = time.Since(start).Milliseconds()
				return res
			}
		}
	}

	observeMs := s.ObserveMs
	if opts.ObserveOverrideMs > 0 {
		observeMs = opts.ObserveOverrideMs
	}
	if observeMs <= 0 {
		observeMs = 500
	}
	if !sleepCtx(ctx, time.Duration(observeMs)*time.Millisecond) {
		res.Status = StratStatusError
		res.Error = ctx.Err().Error()
		res.ElapsedMs = time.Since(start).Milliseconds()
		return res
	}

	// Diff frame ring: anything that arrived after preFrames is "new".
	postFrames := globalBusState.copyRecentFrames()
	newFrames := diffFrames(preFrames, postFrames)
	classify(&res, newFrames, lastExpectedReply)

	res.ElapsedMs = time.Since(start).Milliseconds()
	return res
}

func executeStep(conn *cec.Connection, st Step) error {
	switch st.Kind {
	case StepSendUserControl:
		if !st.Target.IsValid() {
			return fmt.Errorf("invalid target for SendUserControl")
		}
		if err := conn.SendKeypress(st.Target, st.Key, st.Wait); err != nil {
			return err
		}
		if st.HoldMs > 0 {
			time.Sleep(time.Duration(st.HoldMs) * time.Millisecond)
			return conn.SendKeyRelease(st.Target, st.Wait)
		}
		return nil
	case StepTransmit:
		return conn.Transmit(&cec.Command{
			Initiator:   adapterOwnAddress(conn),
			Destination: st.Target,
			Opcode:      st.Opcode,
			OpcodeSet:   true,
			Parameters:  st.Params,
		})
	case StepLibcecVolumeUp:
		return conn.VolumeUp(true)
	case StepLibcecVolumeDown:
		return conn.VolumeDown(true)
	case StepLibcecMute:
		return conn.AudioToggleMute()
	case StepLibcecPowerOn:
		if !st.Target.IsValid() {
			return fmt.Errorf("invalid target for LibcecPowerOn")
		}
		return conn.PowerOn(st.Target)
	case StepLibcecStandby:
		if !st.Target.IsValid() {
			return fmt.Errorf("invalid target for LibcecStandby")
		}
		return conn.Standby(st.Target)
	case StepEnableSAM:
		return conn.SetSystemAudioMode(true)
	case StepWait:
		// handled inline by executeStrategy
		return nil
	default:
		return fmt.Errorf("unknown step kind %d", st.Kind)
	}
}

// adapterOwnAddress returns the adapter's first logical address, falling
// back to LogicalAddressFreeUse so transmits never fail solely on
// initiator selection.
func adapterOwnAddress(conn *cec.Connection) cec.LogicalAddress {
	addrs := conn.GetLogicalAddresses()
	if len(addrs) > 0 {
		return addrs[0]
	}
	return cec.LogicalAddressFreeUse
}

// classify inspects the bus frames that arrived during ObserveMs and labels
// the result.
func classify(res *StratResult, newFrames []BusFrameEntry, expectedReply cec.Opcode) {
	for _, f := range newFrames {
		// Skip our own outbound frames (the executor's transmits show up
		// in the ring too).
		if f.Initiator == int(adapter.Conn().GetLogicalAddresses()[0]) {
			continue
		}
		op := opcodeFromHex(f.Opcode)
		if op == cec.OpcodeFeatureAbort && len(f.ParamsHex) >= 1 {
			res.Status = StratStatusFeatureAborted
			res.AbortOpcode = parseHexByte(f.ParamsHex[0])
			res.ReplyOpcode = int(op)
			res.ReplyName = "FEATURE_ABORT"
			return
		}
		if expectedReply != 0 && op == expectedReply {
			res.Status = StratStatusOK
			res.ReplyOpcode = int(op)
			res.ReplyName = opcodeName(op)
			return
		}
	}
	if !res.Acked {
		res.Status = StratStatusNoAck
		return
	}
	res.Status = StratStatusAckedNoReply
}

func opcodeFromHex(s string) cec.Opcode {
	if len(s) >= 4 && (s[:2] == "0x" || s[:2] == "0X") {
		s = s[2:]
	}
	v := parseHexByte(s)
	return cec.Opcode(v)
}

func parseHexByte(s string) int {
	n := 0
	for _, c := range s {
		d := 0
		switch {
		case c >= '0' && c <= '9':
			d = int(c - '0')
		case c >= 'a' && c <= 'f':
			d = int(c-'a') + 10
		case c >= 'A' && c <= 'F':
			d = int(c-'A') + 10
		default:
			return 0
		}
		n = n*16 + d
	}
	return n & 0xFF
}

// opcodeName returns the named constant for common opcodes; falls back to
// hex. Used for human-readable strategy results in /api/dev.
func opcodeName(op cec.Opcode) string {
	for _, n := range opcodeNames {
		if n.op == op {
			return n.name
		}
	}
	return fmt.Sprintf("0x%02X", op)
}

// stepKindString maps a StepKind to a stable JSON string.
func stepKindString(k StepKind) string {
	switch k {
	case StepSendUserControl:
		return "send_user_control"
	case StepTransmit:
		return "transmit"
	case StepLibcecVolumeUp:
		return "libcec_volume_up"
	case StepLibcecVolumeDown:
		return "libcec_volume_down"
	case StepLibcecMute:
		return "libcec_mute"
	case StepLibcecPowerOn:
		return "libcec_power_on"
	case StepLibcecStandby:
		return "libcec_standby"
	case StepEnableSAM:
		return "enable_sam"
	case StepWait:
		return "wait"
	default:
		return "unknown"
	}
}

// diffFrames returns frames present in post that aren't in pre, by index
// position. Cheap and correct since the frame ring is append-only between
// executor calls (we never race with another executor on the same conn).
func diffFrames(pre, post []BusFrameEntry) []BusFrameEntry {
	if len(post) <= len(pre) {
		return nil
	}
	return post[len(pre):]
}

// copyRecentFrames returns a snapshot of the current frame ring.
func (s *busStateStore) copyRecentFrames() []BusFrameEntry {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := make([]BusFrameEntry, len(s.frameRing))
	copy(out, s.frameRing)
	return out
}

// opcodeName entry table - just the most common reply opcodes for now.
var opcodeNames = []struct {
	op   cec.Opcode
	name string
}{
	{cec.OpcodeReportPowerStatus, "REPORT_POWER_STATUS"},
	{cec.OpcodeReportAudioStatus, "REPORT_AUDIO_STATUS"},
	{cec.OpcodeReportPhysicalAddress, "REPORT_PHYSICAL_ADDRESS"},
	{cec.OpcodeDeviceVendorID, "DEVICE_VENDOR_ID"},
	{cec.OpcodeSetOSDName, "SET_OSD_NAME"},
	{cec.OpcodeCECVersion, "CEC_VERSION"},
	{cec.OpcodeFeatureAbort, "FEATURE_ABORT"},
	{cec.OpcodeActiveSource, "ACTIVE_SOURCE"},
	{cec.OpcodeRoutingChange, "ROUTING_CHANGE"},
	{cec.OpcodeRoutingInformation, "ROUTING_INFORMATION"},
	{cec.OpcodeSetStreamPath, "SET_STREAM_PATH"},
	{cec.OpcodeMenuStatus, "MENU_STATUS"},
	{cec.OpcodeSystemAudioModeStatus, "SYSTEM_AUDIO_MODE_STATUS"},
	{cec.OpcodeReportAudioStatus, "REPORT_AUDIO_STATUS"},
	{cec.OpcodeStandby, "STANDBY"},
	{cec.OpcodeImageViewOn, "IMAGE_VIEW_ON"},
	{cec.OpcodeTextViewOn, "TEXT_VIEW_ON"},
	{cec.OpcodeUserControlPressed, "USER_CONTROL_PRESSED"},
	{cec.OpcodeUserControlReleased, "USER_CONTROL_RELEASED"},
}

// defaultRegistry is the process-wide strategy registry. Initialized in
// init so it's available to handlers and the supervisor.
var defaultRegistry = NewRegistry()
