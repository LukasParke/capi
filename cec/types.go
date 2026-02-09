package cec

/*
#include <libcec/cecc.h>
*/
import "C"

// LogicalAddress represents a CEC logical address (0-15)
type LogicalAddress uint8

const (
	LogicalAddressUnknown          LogicalAddress = 0xF
	LogicalAddressTV               LogicalAddress = 0x0
	LogicalAddressRecordingDevice1 LogicalAddress = 0x1
	LogicalAddressRecordingDevice2 LogicalAddress = 0x2
	LogicalAddressTuner1           LogicalAddress = 0x3
	LogicalAddressPlaybackDevice1  LogicalAddress = 0x4
	LogicalAddressAudioSystem      LogicalAddress = 0x5
	LogicalAddressTuner2           LogicalAddress = 0x6
	LogicalAddressTuner3           LogicalAddress = 0x7
	LogicalAddressPlaybackDevice2  LogicalAddress = 0x8
	LogicalAddressRecordingDevice3 LogicalAddress = 0x9
	LogicalAddressTuner4           LogicalAddress = 0xA
	LogicalAddressPlaybackDevice3  LogicalAddress = 0xB
	LogicalAddressReserved1        LogicalAddress = 0xC
	LogicalAddressReserved2        LogicalAddress = 0xD
	LogicalAddressFreeUse          LogicalAddress = 0xE
	LogicalAddressBroadcast        LogicalAddress = 0xF
)

func (l LogicalAddress) String() string {
	switch l {
	case LogicalAddressTV:
		return "TV"
	case LogicalAddressRecordingDevice1:
		return "Recording Device 1"
	case LogicalAddressRecordingDevice2:
		return "Recording Device 2"
	case LogicalAddressTuner1:
		return "Tuner 1"
	case LogicalAddressPlaybackDevice1:
		return "Playback Device 1"
	case LogicalAddressAudioSystem:
		return "Audio System"
	case LogicalAddressTuner2:
		return "Tuner 2"
	case LogicalAddressTuner3:
		return "Tuner 3"
	case LogicalAddressPlaybackDevice2:
		return "Playback Device 2"
	case LogicalAddressRecordingDevice3:
		return "Recording Device 3"
	case LogicalAddressTuner4:
		return "Tuner 4"
	case LogicalAddressPlaybackDevice3:
		return "Playback Device 3"
	case LogicalAddressBroadcast:
		return "Broadcast"
	default:
		return "Unknown"
	}
}

// DeviceType represents a CEC device type
type DeviceType uint8

const (
	DeviceTypeTV              DeviceType = 0
	DeviceTypeRecordingDevice DeviceType = 1
	DeviceTypeReserved        DeviceType = 2
	DeviceTypeTuner           DeviceType = 3
	DeviceTypePlaybackDevice  DeviceType = 4
	DeviceTypeAudioSystem     DeviceType = 5
)

func (d DeviceType) String() string {
	switch d {
	case DeviceTypeTV:
		return "TV"
	case DeviceTypeRecordingDevice:
		return "Recording Device"
	case DeviceTypeTuner:
		return "Tuner"
	case DeviceTypePlaybackDevice:
		return "Playback Device"
	case DeviceTypeAudioSystem:
		return "Audio System"
	default:
		return "Reserved"
	}
}

// PowerStatus represents device power status
type PowerStatus uint8

const (
	PowerStatusOn               PowerStatus = 0x00
	PowerStatusStandby          PowerStatus = 0x01
	PowerStatusInTransitionStandbyToOn PowerStatus = 0x02
	PowerStatusInTransitionOnToStandby PowerStatus = 0x03
	PowerStatusUnknown          PowerStatus = 0xFF
)

func (p PowerStatus) String() string {
	switch p {
	case PowerStatusOn:
		return "On"
	case PowerStatusStandby:
		return "Standby"
	case PowerStatusInTransitionStandbyToOn:
		return "Transitioning to On"
	case PowerStatusInTransitionOnToStandby:
		return "Transitioning to Standby"
	default:
		return "Unknown"
	}
}

// CECVersion represents CEC version
type CECVersion uint8

const (
	CECVersionUnknown CECVersion = 0x00
	CECVersion1_2     CECVersion = 0x01
	CECVersion1_2A    CECVersion = 0x02
	CECVersion1_3     CECVersion = 0x03
	CECVersion1_3A    CECVersion = 0x04
	CECVersion1_4     CECVersion = 0x05
)

func (v CECVersion) String() string {
	switch v {
	case CECVersion1_2:
		return "1.2"
	case CECVersion1_2A:
		return "1.2a"
	case CECVersion1_3:
		return "1.3"
	case CECVersion1_3A:
		return "1.3a"
	case CECVersion1_4:
		return "1.4"
	default:
		return "Unknown"
	}
}

// Opcode represents a CEC opcode
type Opcode uint8

const (
	OpcodeActiveSource               Opcode = 0x82
	OpcodeImageViewOn                Opcode = 0x04
	OpcodeTextViewOn                 Opcode = 0x0D
	OpcodeInactiveSource             Opcode = 0x9D
	OpcodeRequestActiveSource        Opcode = 0x85
	OpcodeRoutingChange              Opcode = 0x80
	OpcodeRoutingInformation         Opcode = 0x81
	OpcodeSetStreamPath              Opcode = 0x86
	OpcodeStandby                    Opcode = 0x36
	OpcodeRecordOff                  Opcode = 0x0B
	OpcodeRecordOn                   Opcode = 0x09
	OpcodeRecordStatus               Opcode = 0x0A
	OpcodeRecordTVScreen             Opcode = 0x0F
	OpcodeClearAnalogueTimer         Opcode = 0x33
	OpcodeClearDigitalTimer          Opcode = 0x99
	OpcodeClearExternalTimer         Opcode = 0xA1
	OpcodeSetAnalogueTimer           Opcode = 0x34
	OpcodeSetDigitalTimer            Opcode = 0x97
	OpcodeSetExternalTimer           Opcode = 0xA2
	OpcodeSetTimerProgramTitle       Opcode = 0x67
	OpcodeTimerClearedStatus         Opcode = 0x43
	OpcodeTimerStatus                Opcode = 0x35
	OpcodeCECVersion                 Opcode = 0x9E
	OpcodeGetCECVersion              Opcode = 0x9F
	OpcodeGivePhysicalAddress        Opcode = 0x83
	OpcodeGetMenuLanguage            Opcode = 0x91
	OpcodeReportPhysicalAddress      Opcode = 0x84
	OpcodeSetMenuLanguage            Opcode = 0x32
	OpcodeDeckControl                Opcode = 0x42
	OpcodeDeckStatus                 Opcode = 0x1B
	OpcodeGiveDeckStatus             Opcode = 0x1A
	OpcodePlay                       Opcode = 0x41
	OpcodeGiveTunerDeviceStatus      Opcode = 0x08
	OpcodeSelectAnalogueService      Opcode = 0x92
	OpcodeSelectDigitalService       Opcode = 0x93
	OpcodeTunerDeviceStatus          Opcode = 0x07
	OpcodeTunerStepDecrement         Opcode = 0x06
	OpcodeTunerStepIncrement         Opcode = 0x05
	OpcodeDeviceVendorID             Opcode = 0x87
	OpcodeGiveDeviceVendorID         Opcode = 0x8C
	OpcodeVendorCommand              Opcode = 0x89
	OpcodeVendorCommandWithID        Opcode = 0xA0
	OpcodeVendorRemoteButtonDown     Opcode = 0x8A
	OpcodeVendorRemoteButtonUp       Opcode = 0x8B
	OpcodeSetOSDString               Opcode = 0x64
	OpcodeGiveOSDName                Opcode = 0x46
	OpcodeSetOSDName                 Opcode = 0x47
	OpcodeMenuRequest                Opcode = 0x8D
	OpcodeMenuStatus                 Opcode = 0x8E
	OpcodeUserControlPressed         Opcode = 0x44
	OpcodeUserControlReleased        Opcode = 0x45
	OpcodeGiveDevicePowerStatus      Opcode = 0x8F
	OpcodeReportPowerStatus          Opcode = 0x90
	OpcodeFeatureAbort               Opcode = 0x00
	OpcodeAbort                      Opcode = 0xFF
	OpcodeGiveAudioStatus            Opcode = 0x71
	OpcodeGiveSystemAudioModeStatus  Opcode = 0x7D
	OpcodeReportAudioStatus          Opcode = 0x7A
	OpcodeSetSystemAudioMode         Opcode = 0x72
	OpcodeSystemAudioModeRequest     Opcode = 0x70
	OpcodeSystemAudioModeStatus      Opcode = 0x7E
	OpcodeSetAudioRate               Opcode = 0x9A
)

// Keycode represents CEC user control codes
type Keycode uint8

const (
	KeycodeSelect                   Keycode = 0x00
	KeycodeUp                       Keycode = 0x01
	KeycodeDown                     Keycode = 0x02
	KeycodeLeft                     Keycode = 0x03
	KeycodeRight                    Keycode = 0x04
	KeycodeRightUp                  Keycode = 0x05
	KeycodeRightDown                Keycode = 0x06
	KeycodeLeftUp                   Keycode = 0x07
	KeycodeLeftDown                 Keycode = 0x08
	KeycodeRootMenu                 Keycode = 0x09
	KeycodeSetupMenu                Keycode = 0x0A
	KeycodeContentsMenu             Keycode = 0x0B
	KeycodeFavoriteMenu             Keycode = 0x0C
	KeycodeExit                     Keycode = 0x0D
	Keycode0                        Keycode = 0x20
	Keycode1                        Keycode = 0x21
	Keycode2                        Keycode = 0x22
	Keycode3                        Keycode = 0x23
	Keycode4                        Keycode = 0x24
	Keycode5                        Keycode = 0x25
	Keycode6                        Keycode = 0x26
	Keycode7                        Keycode = 0x27
	Keycode8                        Keycode = 0x28
	Keycode9                        Keycode = 0x29
	KeycodeDot                      Keycode = 0x2A
	KeycodeEnter                    Keycode = 0x2B
	KeycodeClear                    Keycode = 0x2C
	KeycodeChannelUp                Keycode = 0x30
	KeycodeChannelDown              Keycode = 0x31
	KeycodePreviousChannel          Keycode = 0x32
	KeycodeSoundSelect              Keycode = 0x33
	KeycodeInputSelect              Keycode = 0x34
	KeycodeDisplayInformation       Keycode = 0x35
	KeycodeHelp                     Keycode = 0x36
	KeycodePageUp                   Keycode = 0x37
	KeycodePageDown                 Keycode = 0x38
	KeycodePower                    Keycode = 0x40
	KeycodeVolumeUp                 Keycode = 0x41
	KeycodeVolumeDown               Keycode = 0x42
	KeycodeMute                     Keycode = 0x43
	KeycodePlay                     Keycode = 0x44
	KeycodeStop                     Keycode = 0x45
	KeycodePause                    Keycode = 0x46
	KeycodeRecord                   Keycode = 0x47
	KeycodeRewind                   Keycode = 0x48
	KeycodeFastForward              Keycode = 0x49
	KeycodeEject                    Keycode = 0x4A
	KeycodeForward                  Keycode = 0x4B
	KeycodeBackward                 Keycode = 0x4C
	KeycodeAngle                    Keycode = 0x50
	KeycodeSubpicture               Keycode = 0x51
	KeycodeF1Blue                   Keycode = 0x71
	KeycodeF2Red                    Keycode = 0x72
	KeycodeF3Green                  Keycode = 0x73
	KeycodeF4Yellow                 Keycode = 0x74
	KeycodeF5                       Keycode = 0x75
)

// DisplayControl represents OSD display duration
type DisplayControl uint8

const (
	DisplayControlDefaultTime    DisplayControl = 0x00
	DisplayControlUntilCleared   DisplayControl = 0x40
	DisplayControlClearPrevious  DisplayControl = 0x80
	DisplayControlReserved       DisplayControl = 0xC0
)

// MenuState represents menu state
type MenuState uint8

const (
	MenuStateActivated   MenuState = 0x00
	MenuStateDeactivated MenuState = 0x01
)

// LogLevel represents log message level
type LogLevel int

const (
	LogLevelError   LogLevel = 1
	LogLevelWarning LogLevel = 2
	LogLevelNotice  LogLevel = 4
	LogLevelTraffic LogLevel = 8
	LogLevelDebug   LogLevel = 16
	LogLevelAll     LogLevel = 31
)

func (l LogLevel) String() string {
	switch l {
	case LogLevelError:
		return "ERROR"
	case LogLevelWarning:
		return "WARNING"
	case LogLevelNotice:
		return "NOTICE"
	case LogLevelTraffic:
		return "TRAFFIC"
	case LogLevelDebug:
		return "DEBUG"
	default:
		return "ALL"
	}
}

// Alert represents CEC alert type
type Alert int

const (
	AlertServiceDevice           Alert = 1
	AlertConnectionLost          Alert = 2
	AlertPermissionError         Alert = 3
	AlertPortBusy                Alert = 4
	AlertPhysicalAddressError    Alert = 5
	AlertTVPollFailed            Alert = 6
)

// Parameter represents alert parameter
type Parameter struct {
	Type  int
	Value int64
}

// Command represents a CEC command
type Command struct {
	Initiator    LogicalAddress
	Destination  LogicalAddress
	Ack          bool
	Eom          bool
	Opcode       Opcode
	OpcodeSet    bool
	Parameters   []uint8
	TransmitTime int64
}

// Adapter represents a CEC adapter
type Adapter struct {
	Path string
	Comm string
}

// Device represents a CEC device with all its properties
type Device struct {
	LogicalAddress  LogicalAddress
	PhysicalAddress uint16
	VendorID        uint64
	CECVersion      CECVersion
	PowerStatus     PowerStatus
	OSDName         string
	MenuLanguage    string
	IsActive        bool
	IsActiveSource  bool
}
