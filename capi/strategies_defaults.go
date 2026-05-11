package main

import (
	"github.com/LukasParke/capi/cec"
)

// defaultStrategies returns the built-in strategy chains used when no
// per-vendor override is registered. Each chain lists strategies in
// preference order; the executor stops at the first one whose observed
// outcome is "ok" (unless RunOptions.AllStrategies = true, used by the
// strategy bench).
//
// Design notes:
//   - Defaults err on the side of "try the cheap thing first, then the
//     thing that requires AVR/SAM cooperation, then libcec's best-effort".
//   - The Target field is left as LogicalAddressUnknown for keys that
//     should follow the active source; the executor substitutes
//     RunOptions.Target.
//   - Hold durations: 250ms for volume keys (most AVRs/TVs need that to
//     register), 100ms for nav/menu keys (snappier), 0 for one-shot
//     transport keys.
func defaultStrategies() map[Action][]Strategy {
	const (
		volHold = 250
		navHold = 100
	)

	ucPress := func(name string, target cec.LogicalAddress, key cec.Keycode, hold int) Strategy {
		return Strategy{
			Name: name,
			Steps: []Step{{
				Kind:   StepSendUserControl,
				Target: target,
				Key:    key,
				Wait:   true,
				HoldMs: hold,
			}},
		}
	}

	ucPressActive := func(name string, key cec.Keycode, hold int) Strategy {
		return Strategy{
			Name: name,
			Steps: []Step{{
				Kind:   StepSendUserControl,
				Target: cec.LogicalAddressUnknown, // executor substitutes opts.Target
				Key:    key,
				Wait:   true,
				HoldMs: hold,
			}},
		}
	}

	libcecVolumeStrategy := func(name string, kind StepKind) Strategy {
		return Strategy{
			Name:  name,
			Steps: []Step{{Kind: kind}},
		}
	}

	return map[Action][]Strategy{
		// Volume actions: try AudioSystem, then TV, then Playback1, then
		// libcec_volume_*. EnableSAM is intentionally NOT in defaults; it
		// changes bus state and should be opted into per-vendor.
		ActVolumeUp: {
			ucPress("uc_volume_up_audio", cec.LogicalAddressAudioSystem, cec.KeycodeVolumeUp, volHold),
			ucPress("uc_volume_up_tv", cec.LogicalAddressTV, cec.KeycodeVolumeUp, volHold),
			ucPress("uc_volume_up_playback1", cec.LogicalAddressPlaybackDevice1, cec.KeycodeVolumeUp, volHold),
			libcecVolumeStrategy("libcec_volume_up", StepLibcecVolumeUp),
		},
		ActVolumeDown: {
			ucPress("uc_volume_down_audio", cec.LogicalAddressAudioSystem, cec.KeycodeVolumeDown, volHold),
			ucPress("uc_volume_down_tv", cec.LogicalAddressTV, cec.KeycodeVolumeDown, volHold),
			ucPress("uc_volume_down_playback1", cec.LogicalAddressPlaybackDevice1, cec.KeycodeVolumeDown, volHold),
			libcecVolumeStrategy("libcec_volume_down", StepLibcecVolumeDown),
		},
		ActMute: {
			ucPress("uc_mute_audio", cec.LogicalAddressAudioSystem, cec.KeycodeMute, volHold),
			ucPress("uc_mute_tv", cec.LogicalAddressTV, cec.KeycodeMute, volHold),
			libcecVolumeStrategy("libcec_mute", StepLibcecMute),
		},

		// Nav keys default to "send to opts.Target" with a fallback of
		// "send to active source" - same chain since we don't know the
		// active source LA without observation. The executor's
		// RunOptions.Target is the one input that decides this.
		ActNavUp:    {ucPressActive("uc_up_target", cec.KeycodeUp, navHold)},
		ActNavDown:  {ucPressActive("uc_down_target", cec.KeycodeDown, navHold)},
		ActNavLeft:  {ucPressActive("uc_left_target", cec.KeycodeLeft, navHold)},
		ActNavRight: {ucPressActive("uc_right_target", cec.KeycodeRight, navHold)},
		ActSelect:   {ucPressActive("uc_select_target", cec.KeycodeSelect, navHold)},
		ActBack:     {ucPressActive("uc_back_target", cec.KeycodeExit, navHold)},
		ActHome:     {ucPressActive("uc_home_target", cec.KeycodeRootMenu, navHold)},
		ActMenu:     {ucPressActive("uc_menu_target", cec.KeycodeSetupMenu, navHold)},

		// Channel keys default to Tuner1 then active source.
		ActChannelUp: {
			ucPress("uc_channel_up_tuner1", cec.LogicalAddressTuner1, cec.KeycodeChannelUp, navHold),
			ucPressActive("uc_channel_up_target", cec.KeycodeChannelUp, navHold),
		},
		ActChannelDown: {
			ucPress("uc_channel_down_tuner1", cec.LogicalAddressTuner1, cec.KeycodeChannelDown, navHold),
			ucPressActive("uc_channel_down_target", cec.KeycodeChannelDown, navHold),
		},

		// Playback keys go to the target.
		ActPlay:        {ucPressActive("uc_play_target", cec.KeycodePlay, 0)},
		ActPause:       {ucPressActive("uc_pause_target", cec.KeycodePause, 0)},
		ActStop:        {ucPressActive("uc_stop_target", cec.KeycodeStop, 0)},
		ActFastForward: {ucPressActive("uc_ff_target", cec.KeycodeFastForward, 0)},
		ActRewind:      {ucPressActive("uc_rew_target", cec.KeycodeRewind, 0)},
		ActRecord:      {ucPressActive("uc_record_target", cec.KeycodeRecord, 0)},

		// Power actions use libcec, then fall back to UC Power direct to target.
		ActPower: {
			Strategy{
				Name:  "libcec_power_on_target",
				Steps: []Step{{Kind: StepLibcecPowerOn, Target: cec.LogicalAddressUnknown}},
			},
			ucPressActive("uc_power_target", cec.KeycodePower, navHold),
		},

		// Number keys.
		ActNumber0: {ucPressActive("uc_0_target", cec.Keycode0, navHold)},
		ActNumber1: {ucPressActive("uc_1_target", cec.Keycode1, navHold)},
		ActNumber2: {ucPressActive("uc_2_target", cec.Keycode2, navHold)},
		ActNumber3: {ucPressActive("uc_3_target", cec.Keycode3, navHold)},
		ActNumber4: {ucPressActive("uc_4_target", cec.Keycode4, navHold)},
		ActNumber5: {ucPressActive("uc_5_target", cec.Keycode5, navHold)},
		ActNumber6: {ucPressActive("uc_6_target", cec.Keycode6, navHold)},
		ActNumber7: {ucPressActive("uc_7_target", cec.Keycode7, navHold)},
		ActNumber8: {ucPressActive("uc_8_target", cec.Keycode8, navHold)},
		ActNumber9: {ucPressActive("uc_9_target", cec.Keycode9, navHold)},
	}
}
