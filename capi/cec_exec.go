package main

import (
	"fmt"
	"strconv"

	"github.com/LukasParke/capi/cec"
)

// CEC command helpers shared by JSON API handlers and HTMX UI actions.
//
// Each helper acquires the live adapter via adapter.With and lets the
// cec package's internal serialization handle libcec safety. The Adapter
// returns ErrAdapterUnavailable when no session is open, which the caller
// surfaces as a 503.

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
	var msg string
	err := adapter.With(func(c *cec.Connection) error {
		if addrStr != "" {
			addr, err := strconv.Atoi(addrStr)
			if err != nil || addr < 0 || addr > 15 {
				return fmt.Errorf("invalid address")
			}
			if err := c.SendVolumeKey(cec.LogicalAddress(addr), cec.KeycodeVolumeUp); err != nil {
				return err
			}
			msg = fmt.Sprintf("Volume up sent to device %d", addr)
			return nil
		}
		if err := c.VolumeUpBestEffort(true); err != nil {
			return err
		}
		msg = "Volume up command sent"
		return nil
	})
	if err == nil {
		schedulePostCommandBusRefresh()
	}
	return msg, err
}

func execVolumeDown(addrStr string) (string, error) {
	var msg string
	err := adapter.With(func(c *cec.Connection) error {
		if addrStr != "" {
			addr, err := strconv.Atoi(addrStr)
			if err != nil || addr < 0 || addr > 15 {
				return fmt.Errorf("invalid address")
			}
			if err := c.SendVolumeKey(cec.LogicalAddress(addr), cec.KeycodeVolumeDown); err != nil {
				return err
			}
			msg = fmt.Sprintf("Volume down sent to device %d", addr)
			return nil
		}
		if err := c.VolumeDownBestEffort(true); err != nil {
			return err
		}
		msg = "Volume down command sent"
		return nil
	})
	if err == nil {
		schedulePostCommandBusRefresh()
	}
	return msg, err
}

func execVolumeMute(addrStr string) (string, error) {
	var msg string
	err := adapter.With(func(c *cec.Connection) error {
		if addrStr != "" {
			addr, err := strconv.Atoi(addrStr)
			if err != nil || addr < 0 || addr > 15 {
				return fmt.Errorf("invalid address")
			}
			if err := c.SendVolumeKey(cec.LogicalAddress(addr), cec.KeycodeMute); err != nil {
				return err
			}
			msg = fmt.Sprintf("Mute sent to device %d", addr)
			return nil
		}
		if err := c.MuteBestEffort(); err != nil {
			return err
		}
		msg = "Mute toggle command sent"
		return nil
	})
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
// keycodes used by both the HTTP API and the HTMX UI.
var keyNameMap = map[string]cec.Keycode{
	"up":     cec.KeycodeUp,
	"down":   cec.KeycodeDown,
	"left":   cec.KeycodeLeft,
	"right":  cec.KeycodeRight,
	"select": cec.KeycodeSelect,
	"enter":  cec.KeycodeEnter,
	"back":   cec.KeycodeExit,
	"home":   cec.KeycodeRootMenu,
	"menu":   cec.KeycodeSetupMenu,
	"play":   cec.KeycodePlay,
	"pause":  cec.KeycodePause,
	"stop":   cec.KeycodeStop,
}

func execSendKey(addr int, keyName string, keycode int) error {
	if addr < 0 || addr > 15 {
		return fmt.Errorf("invalid logical address")
	}
	var keycodeVal cec.Keycode
	if keyName != "" {
		k, ok := keyNameMap[keyName]
		if !ok {
			return fmt.Errorf("unsupported key name")
		}
		keycodeVal = k
	} else {
		if keycode < 0 || keycode > 0xFF {
			return fmt.Errorf("keycode must be in range 0-255")
		}
		keycodeVal = cec.Keycode(keycode)
	}
	err := adapter.With(func(c *cec.Connection) error {
		return c.SendButton(cec.LogicalAddress(addr), keycodeVal)
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
