package main

import (
	"context"
	"fmt"
	"strconv"
	"time"

	"github.com/LukasParke/capi/cec"
)

// CEC command helpers shared by JSON API handlers and HTMX UI actions.
//
// Each helper acquires the live adapter via adapter.With and lets the
// cec package's internal serialization handle libcec safety. The Adapter
// returns ErrAdapterUnavailable when no session is open, which the caller
// surfaces as a 503.
//
// Helpers that map to a registry Action (volume / mute / nav / numbers /
// playback / channel / power) drive defaultRegistry instead of directly
// calling cec helpers, so per-vendor overrides apply uniformly to JSON,
// MQTT, and HTMX entry points.

// runAction executes the strategy chain for an action, returning a
// short human-readable message and the last error if every strategy failed.
// If target is LogicalAddressUnknown, the registry's per-strategy targets
// (or active source for nav) are used directly.
func runAction(action Action, target cec.LogicalAddress) (string, error) {
	c := adapter.Conn()
	if c == nil {
		return "", fmt.Errorf("CEC adapter not available")
	}
	vendor := vendorIDForTarget(target)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	results, err := defaultRegistry.Run(ctx, c, action, RunOptions{
		Vendor: vendor,
		Target: target,
	})
	if err != nil {
		return "", err
	}
	for _, r := range results {
		if r.Status == StratStatusOK {
			return fmt.Sprintf("%s ok via %s (reply %s)", action, r.Strategy, r.ReplyName), nil
		}
	}
	if len(results) == 0 {
		return "", fmt.Errorf("no strategies for %s", action)
	}
	last := results[len(results)-1]
	return fmt.Sprintf("%s tried %d strategies; last status=%s strategy=%s",
		action, len(results), last.Status, last.Strategy), nil
}

// vendorIDForTarget looks up the vendor ID we've observed for a destination
// LA from the steward snapshot. Returns empty string when unknown so the
// registry uses defaults.
func vendorIDForTarget(target cec.LogicalAddress) string {
	if !target.IsValid() {
		return ""
	}
	snap := globalBusState.copySnapshot()
	for _, dm := range snap.Devices {
		if la, ok := dm["logical_address"].(int); ok && la == int(target) {
			if v, ok := dm["vendor_id"].(string); ok {
				return v
			}
		}
	}
	return ""
}

func execPowerOn(addr int) error {
	if addr < 0 || addr > 15 {
		return fmt.Errorf("invalid logical address")
	}
	err := adapter.With(func(c *cec.Connection) error {
		return c.PowerOn(cec.LogicalAddress(addr))
	})
	if err == nil {
		schedulePostCommandBusRefresh()
	}
	return err
}

func execPowerOff(addr int) error {
	if addr < 0 || addr > 15 {
		return fmt.Errorf("invalid logical address")
	}
	err := adapter.With(func(c *cec.Connection) error {
		return c.Standby(cec.LogicalAddress(addr))
	})
	if err == nil {
		schedulePostCommandBusRefresh()
	}
	return err
}

func execPowerStatus(addr int) (string, error) {
	if addr < 0 || addr > 15 {
		return "", fmt.Errorf("invalid logical address")
	}
	var out string
	err := adapter.With(func(c *cec.Connection) error {
		st, err := c.GetDevicePowerStatus(cec.LogicalAddress(addr))
		if err != nil {
			return err
		}
		out = st.String()
		return nil
	})
	return out, err
}

func execVolumeUp(addrStr string) (string, error) {
	return runVolumeAction(ActVolumeUp, addrStr)
}

func execVolumeDown(addrStr string) (string, error) {
	return runVolumeAction(ActVolumeDown, addrStr)
}

func execVolumeMute(addrStr string) (string, error) {
	return runVolumeAction(ActMute, addrStr)
}

// runVolumeAction is the shared body for all volume helpers. Empty addrStr
// runs the default chain (which targets AudioSystem then TV then libcec);
// a numeric addrStr forces the action at that LA via the registry's
// per-target Run(opts.Target).
func runVolumeAction(action Action, addrStr string) (string, error) {
	target := cec.LogicalAddressUnknown
	if addrStr != "" {
		addr, err := strconv.Atoi(addrStr)
		if err != nil || addr < 0 || addr > 14 {
			return "", fmt.Errorf("invalid address")
		}
		target = cec.LogicalAddress(addr)
	}
	msg, err := runAction(action, target)
	if err == nil {
		schedulePostCommandBusRefresh()
	}
	return msg, err
}

func execSetActiveSource(addr int) error {
	if addr < 0 || addr > 15 {
		return fmt.Errorf("invalid logical address")
	}
	err := adapter.With(func(c *cec.Connection) error {
		return c.SwitchToDevice(cec.LogicalAddress(addr))
	})
	if err == nil {
		schedulePostCommandBusRefresh()
	}
	return err
}

func execHDMIPort(port int) error {
	if port < 1 || port > 15 {
		return fmt.Errorf("invalid HDMI port")
	}
	err := adapter.With(func(c *cec.Connection) error {
		return c.SwitchToHDMIPort(uint8(port))
	})
	if err == nil {
		schedulePostCommandBusRefresh()
	}
	return err
}

// keyNameMap is the canonical mapping from human-readable key names to CEC
// keycodes, used by both the HTTP API and the HTMX UI. Names are
// lowercase and underscored; the JSON / form parser normalizes incoming
// values via normalizeKeyName before lookup.
//
// All 60+ named keycodes from the cec package are exposed here (the Phase 2
// expansion); previously only 12 were reachable from the API.
var keyNameMap = map[string]cec.Keycode{
	// Navigation
	"select":     cec.KeycodeSelect,
	"up":         cec.KeycodeUp,
	"down":       cec.KeycodeDown,
	"left":       cec.KeycodeLeft,
	"right":      cec.KeycodeRight,
	"right_up":   cec.KeycodeRightUp,
	"right_down": cec.KeycodeRightDown,
	"left_up":    cec.KeycodeLeftUp,
	"left_down":  cec.KeycodeLeftDown,
	"root_menu":  cec.KeycodeRootMenu,
	"home":       cec.KeycodeRootMenu,
	"setup_menu": cec.KeycodeSetupMenu,
	"menu":       cec.KeycodeSetupMenu,
	"contents_menu": cec.KeycodeContentsMenu,
	"favorite_menu": cec.KeycodeFavoriteMenu,
	"exit":       cec.KeycodeExit,
	"back":       cec.KeycodeExit,
	"enter":      cec.KeycodeEnter,
	"clear":      cec.KeycodeClear,

	// Number pad
	"0": cec.Keycode0, "1": cec.Keycode1, "2": cec.Keycode2,
	"3": cec.Keycode3, "4": cec.Keycode4, "5": cec.Keycode5,
	"6": cec.Keycode6, "7": cec.Keycode7, "8": cec.Keycode8,
	"9":   cec.Keycode9,
	"dot": cec.KeycodeDot,

	// Channels / inputs
	"channel_up":          cec.KeycodeChannelUp,
	"channel_down":        cec.KeycodeChannelDown,
	"previous_channel":    cec.KeycodePreviousChannel,
	"sound_select":        cec.KeycodeSoundSelect,
	"input_select":        cec.KeycodeInputSelect,
	"display_information": cec.KeycodeDisplayInformation,
	"help":                cec.KeycodeHelp,
	"page_up":             cec.KeycodePageUp,
	"page_down":           cec.KeycodePageDown,

	// Power / volume
	"power":       cec.KeycodePower,
	"volume_up":   cec.KeycodeVolumeUp,
	"volume_down": cec.KeycodeVolumeDown,
	"mute":        cec.KeycodeMute,

	// Transport
	"play":         cec.KeycodePlay,
	"stop":         cec.KeycodeStop,
	"pause":        cec.KeycodePause,
	"record":       cec.KeycodeRecord,
	"rewind":       cec.KeycodeRewind,
	"fast_forward": cec.KeycodeFastForward,
	"eject":        cec.KeycodeEject,
	"forward":      cec.KeycodeForward,
	"backward":     cec.KeycodeBackward,
	"angle":        cec.KeycodeAngle,
	"subpicture":   cec.KeycodeSubpicture,

	// Coloured buttons
	"f1_blue":   cec.KeycodeF1Blue,
	"f2_red":    cec.KeycodeF2Red,
	"f3_green":  cec.KeycodeF3Green,
	"f4_yellow": cec.KeycodeF4Yellow,
	"f5":        cec.KeycodeF5,
}

// keyNameToAction maps key names that have a corresponding registry Action,
// so execSendKey can route through the strategy registry (and pick up
// per-vendor overrides) when applicable.
var keyNameToAction = map[string]Action{
	"up":           ActNavUp,
	"down":         ActNavDown,
	"left":         ActNavLeft,
	"right":        ActNavRight,
	"select":       ActSelect,
	"back":         ActBack,
	"exit":         ActBack,
	"home":         ActHome,
	"root_menu":    ActHome,
	"menu":         ActMenu,
	"setup_menu":   ActMenu,
	"play":         ActPlay,
	"pause":        ActPause,
	"stop":         ActStop,
	"fast_forward": ActFastForward,
	"rewind":       ActRewind,
	"record":       ActRecord,
	"power":        ActPower,
	"volume_up":    ActVolumeUp,
	"volume_down":  ActVolumeDown,
	"mute":         ActMute,
	"channel_up":   ActChannelUp,
	"channel_down": ActChannelDown,
	"0":            ActNumber0,
	"1":            ActNumber1,
	"2":            ActNumber2,
	"3":            ActNumber3,
	"4":            ActNumber4,
	"5":            ActNumber5,
	"6":            ActNumber6,
	"7":            ActNumber7,
	"8":            ActNumber8,
	"9":            ActNumber9,
}

// normalizeKeyName converts user-supplied key names (any case, with -, _
// or spaces) to the canonical lowercase-underscored form used by the maps
// above.
func normalizeKeyName(name string) string {
	if name == "" {
		return ""
	}
	out := make([]byte, 0, len(name))
	for i := 0; i < len(name); i++ {
		c := name[i]
		switch {
		case c >= 'A' && c <= 'Z':
			c = c + ('a' - 'A')
		case c == '-' || c == ' ':
			c = '_'
		}
		out = append(out, c)
	}
	return string(out)
}

func execSendKey(addr int, keyName string, keycode int) error {
	if addr < 0 || addr > 14 {
		return fmt.Errorf("invalid logical address")
	}
	target := cec.LogicalAddress(addr)
	norm := normalizeKeyName(keyName)
	if norm != "" {
		// Prefer the registry path when this key has an Action mapping;
		// per-vendor overrides apply uniformly.
		if action, ok := keyNameToAction[norm]; ok {
			_, err := runAction(action, target)
			if err == nil {
				schedulePostCommandBusRefresh()
			}
			return err
		}
		// Otherwise fall through to a direct one-shot SendButton with the
		// keycode from the expanded keymap.
		k, ok := keyNameMap[norm]
		if !ok {
			return fmt.Errorf("unsupported key name %q", keyName)
		}
		err := adapter.With(func(c *cec.Connection) error {
			return c.SendButton(target, k)
		})
		if err == nil {
			schedulePostCommandBusRefresh()
		}
		return err
	}

	if keycode < 0 || keycode > 0xFF {
		return fmt.Errorf("keycode must be in range 0-255")
	}
	err := adapter.With(func(c *cec.Connection) error {
		return c.SendButton(target, cec.Keycode(keycode))
	})
	if err == nil {
		schedulePostCommandBusRefresh()
	}
	return err
}

// execAudioStatusDisplay returns the audio status formatted for UI display.
// Returns zeros when no adapter is attached.
func execAudioStatusDisplay() (displayVol int, muted bool, raw int, volumeRaw int) {
	_ = adapter.With(func(c *cec.Connection) error {
		vol, m, rawB := c.GetAudioStatus()
		displayVol = int(vol)
		if displayVol > 100 {
			displayVol = 100
		}
		muted = m
		raw = int(rawB)
		volumeRaw = int(vol)
		return nil
	})
	return
}
