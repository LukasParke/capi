package cec

/*
#cgo pkg-config: libcec
#include <libcec/cecc.h>
#include <stdlib.h>

// Callback forwarders
extern void goLogMessageCallback(void*, const cec_log_message*);
extern void goKeyPressCallback(void*, const cec_keypress*);
extern void goCommandCallback(void*, const cec_command*);
extern void goConfigurationChangedCallback(void*, const libcec_configuration*);
extern void goAlertCallback(void*, const libcec_alert, const libcec_parameter);
extern int goMenuStateChangedCallback(void*, const cec_menu_state);
extern void goSourceActivatedCallback(void*, const cec_logical_address, const uint8_t);

// Callback struct initialization helper
static ICECCallbacks* createCallbacks() {
    ICECCallbacks* callbacks = (ICECCallbacks*)malloc(sizeof(ICECCallbacks));
    callbacks->logMessage = goLogMessageCallback;
    callbacks->keyPress = goKeyPressCallback;
    callbacks->commandReceived = goCommandCallback;
    callbacks->configurationChanged = goConfigurationChangedCallback;
    callbacks->alert = goAlertCallback;
    callbacks->menuStateChanged = goMenuStateChangedCallback;
    callbacks->sourceActivated = goSourceActivatedCallback;
    return callbacks;
}
*/
import "C"
import (
	"errors"
	"fmt"
	"sync"
	"time"
	"unsafe"
)

// Connection represents a connection to the CEC adapter
type Connection struct {
	handle      C.libcec_connection_t
	config      *Configuration
	callbacks   CallbackHandler
	mu          sync.Mutex
	initialized bool
}

// Configuration holds CEC configuration
type Configuration struct {
	DeviceName        string
	DeviceType        DeviceType
	PhysicalAddress   uint16
	BaseDevice        LogicalAddress
	HDMIPort          uint8
	ClientVersion     uint32
	ServerVersion     uint32
	TryLogicalAddress LogicalAddress
}

// CallbackHandler interface for handling CEC events
type CallbackHandler interface {
	OnLogMessage(level LogLevel, time int64, message string)
	OnKeyPress(key Keycode, duration uint32)
	OnCommand(command *Command)
	OnConfigurationChanged(config *Configuration)
	OnAlert(alert Alert, param Parameter)
	OnMenuStateChanged(state MenuState) bool
	OnSourceActivated(address LogicalAddress, activated bool)
}

// DefaultCallbackHandler provides no-op implementations
type DefaultCallbackHandler struct{}

func (d *DefaultCallbackHandler) OnLogMessage(level LogLevel, time int64, message string)  {}
func (d *DefaultCallbackHandler) OnKeyPress(key Keycode, duration uint32)                  {}
func (d *DefaultCallbackHandler) OnCommand(command *Command)                               {}
func (d *DefaultCallbackHandler) OnConfigurationChanged(config *Configuration)             {}
func (d *DefaultCallbackHandler) OnAlert(alert Alert, param Parameter)                     {}
func (d *DefaultCallbackHandler) OnMenuStateChanged(state MenuState) bool                  { return true }
func (d *DefaultCallbackHandler) OnSourceActivated(address LogicalAddress, activated bool) {}

// Global connection registry for callbacks
var (
	connections   = make(map[C.libcec_connection_t]*Connection)
	connectionsMu sync.RWMutex
)

// Open creates a new CEC connection
func Open(deviceName string, deviceType DeviceType) (*Connection, error) {
	config := &Configuration{
		DeviceName:        deviceName,
		DeviceType:        deviceType,
		PhysicalAddress:   0xFFFF, // Auto-detect
		ClientVersion:     C.LIBCEC_VERSION_CURRENT,
		TryLogicalAddress: LogicalAddressUnknown,
	}
	return OpenWithConfig(config)
}

// OpenWithConfig creates a new CEC connection with custom configuration
func OpenWithConfig(config *Configuration) (*Connection, error) {
	conn := &Connection{
		config:    config,
		callbacks: &DefaultCallbackHandler{},
	}

	// Create libcec configuration
	cConfig := C.libcec_configuration{}
	C.libcec_clear_configuration(&cConfig)

	cDeviceName := C.CString(config.DeviceName)
	defer C.free(unsafe.Pointer(cDeviceName))
	C.strncpy(&cConfig.strDeviceName[0], cDeviceName, 13)

	cConfig.deviceTypes.types[0] = C.cec_device_type(config.DeviceType)
	cConfig.iPhysicalAddress = C.uint16_t(config.PhysicalAddress)
	cConfig.baseDevice = C.cec_logical_address(config.BaseDevice)
	cConfig.iHDMIPort = C.uint8_t(config.HDMIPort)
	cConfig.clientVersion = C.uint32_t(config.ClientVersion)

	// Create callbacks
	callbacks := C.createCallbacks()
	cConfig.callbacks = callbacks

	// Initialize libcec
	conn.handle = C.libcec_initialise(&cConfig)
	if conn.handle == nil {
		return nil, errors.New("failed to initialize libcec")
	}

	// Register connection for callbacks
	connectionsMu.Lock()
	connections[conn.handle] = conn
	connectionsMu.Unlock()

	conn.initialized = true
	return conn, nil
}

// SetCallbackHandler sets the callback handler for events
func (c *Connection) SetCallbackHandler(handler CallbackHandler) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.callbacks = handler
}

// FindAdapters lists available CEC adapters
func (c *Connection) FindAdapters() ([]Adapter, error) {
	var adapters [10]C.cec_adapter
	count := C.libcec_find_adapters(c.handle, &adapters[0], 10, nil)

	if count < 0 {
		return nil, errors.New("failed to find adapters")
	}

	result := make([]Adapter, count)
	for i := 0; i < int(count); i++ {
		result[i] = Adapter{
			Path: C.GoString(&adapters[i].path[0]),
			Comm: C.GoString(&adapters[i].comm[0]),
		}
	}

	return result, nil
}

// OpenAdapter opens a connection to a specific adapter
func (c *Connection) OpenAdapter(adapterPath string) error {
	cPath := C.CString(adapterPath)
	defer C.free(unsafe.Pointer(cPath))

	if C.libcec_open(c.handle, cPath, 5000) == 0 {
		return errors.New("failed to open adapter")
	}

	return nil
}

// Close closes the CEC connection
func (c *Connection) Close() error {
	if !c.initialized {
		return nil
	}

	connectionsMu.Lock()
	delete(connections, c.handle)
	connectionsMu.Unlock()

	C.libcec_close(c.handle)
	C.libcec_destroy(c.handle)
	c.initialized = false

	return nil
}

// PowerOn powers on a device
func (c *Connection) PowerOn(address LogicalAddress) error {
	if C.libcec_power_on_devices(c.handle, C.cec_logical_address(address)) == 0 {
		return fmt.Errorf("failed to power on device %d", address)
	}
	return nil
}

// Standby puts a device in standby mode
func (c *Connection) Standby(address LogicalAddress) error {
	if C.libcec_standby_devices(c.handle, C.cec_logical_address(address)) == 0 {
		return fmt.Errorf("failed to standby device %d", address)
	}
	return nil
}

// SetActiveSource sets the active source
func (c *Connection) SetActiveSource(deviceType DeviceType) error {
	if C.libcec_set_active_source(c.handle, C.cec_device_type(deviceType)) == 0 {
		return errors.New("failed to set active source")
	}
	return nil
}

// SetInactiveView marks as inactive view
func (c *Connection) SetInactiveView() error {
	if C.libcec_set_inactive_view(c.handle) == 0 {
		return errors.New("failed to set inactive view")
	}
	return nil
}

// VolumeUp increases volume
func (c *Connection) VolumeUp(sendRelease bool) error {
	release := C.int(0)
	if sendRelease {
		release = 1
	}
	if C.libcec_volume_up(c.handle, release) == 0 {
		return errors.New("failed to increase volume")
	}
	return nil
}

// VolumeDown decreases volume
func (c *Connection) VolumeDown(sendRelease bool) error {
	release := C.int(0)
	if sendRelease {
		release = 1
	}
	if C.libcec_volume_down(c.handle, release) == 0 {
		return errors.New("failed to decrease volume")
	}
	return nil
}

// AudioToggleMute toggles mute
func (c *Connection) AudioToggleMute() error {
	if C.libcec_audio_toggle_mute(c.handle) == 0 {
		return errors.New("failed to toggle mute")
	}
	return nil
}

// AudioMute mutes audio
func (c *Connection) AudioMute() error {
	if C.libcec_audio_mute(c.handle) == 0 {
		return errors.New("failed to mute audio")
	}
	return nil
}

// AudioUnmute unmutes audio
func (c *Connection) AudioUnmute() error {
	if C.libcec_audio_unmute(c.handle) == 0 {
		return errors.New("failed to unmute audio")
	}
	return nil
}

// GetDevicePowerStatus gets the power status of a device
func (c *Connection) GetDevicePowerStatus(address LogicalAddress) (PowerStatus, error) {
	status := C.libcec_get_device_power_status(c.handle, C.cec_logical_address(address))
	if status == C.CEC_POWER_STATUS_UNKNOWN {
		return PowerStatusUnknown, errors.New("failed to get power status")
	}
	return PowerStatus(status), nil
}

// GetActiveSource gets the active source
func (c *Connection) GetActiveSource() (LogicalAddress, error) {
	addr := C.libcec_get_active_source(c.handle)
	return LogicalAddress(addr), nil
}

// IsActiveSource checks if the specified device is the active source
func (c *Connection) IsActiveSource(address LogicalAddress) bool {
	return C.libcec_is_active_source(c.handle, C.cec_logical_address(address)) == 1
}

// GetDeviceVendorId gets the vendor ID of a device
func (c *Connection) GetDeviceVendorId(address LogicalAddress) (uint64, error) {
	vendorId := C.libcec_get_device_vendor_id(c.handle, C.cec_logical_address(address))
	if vendorId == C.CEC_VENDOR_UNKNOWN {
		return 0, errors.New("failed to get vendor ID")
	}
	return uint64(vendorId), nil
}

// GetDevicePhysicalAddress gets the physical address of a device
func (c *Connection) GetDevicePhysicalAddress(address LogicalAddress) (uint16, error) {
	addr := C.libcec_get_device_physical_address(c.handle, C.cec_logical_address(address))
	if addr == C.CEC_INVALID_PHYSICAL_ADDRESS {
		return 0, errors.New("failed to get physical address")
	}
	return uint16(addr), nil
}

// GetDeviceOSDName gets the OSD name of a device
func (c *Connection) GetDeviceOSDName(address LogicalAddress) (string, error) {
	var name [14]C.char
	if C.libcec_get_device_osd_name(c.handle, C.cec_logical_address(address), &name[0]) == 0 {
		return "", errors.New("failed to get OSD name")
	}
	return C.GoString(&name[0]), nil
}

// GetDeviceMenuLanguage gets the menu language of a device
func (c *Connection) GetDeviceMenuLanguage(address LogicalAddress) (string, error) {
	var lang [4]C.char
	if C.libcec_get_device_menu_language(c.handle, C.cec_logical_address(address), &lang[0]) == 0 {
		return "", errors.New("failed to get menu language")
	}
	return C.GoString(&lang[0]), nil
}

// GetDeviceCecVersion gets the CEC version of a device
func (c *Connection) GetDeviceCecVersion(address LogicalAddress) (CECVersion, error) {
	version := C.libcec_get_device_cec_version(c.handle, C.cec_logical_address(address))
	if version == C.CEC_VERSION_UNKNOWN {
		return CECVersionUnknown, errors.New("failed to get CEC version")
	}
	return CECVersion(version), nil
}

// GetActiveDevices returns a list of active devices
func (c *Connection) GetActiveDevices() []LogicalAddress {
	addresses := C.libcec_get_active_devices(c.handle)

	var result []LogicalAddress
	for i := 0; i < 16; i++ {
		if addresses.addresses[i] != 0 {
			result = append(result, LogicalAddress(i))
		}
	}
	return result
}

// IsActiveDevice checks if a device is active
func (c *Connection) IsActiveDevice(address LogicalAddress) bool {
	return C.libcec_is_active_device(c.handle, C.cec_logical_address(address)) == 1
}

// Transmit sends a raw CEC command
func (c *Connection) Transmit(command *Command) error {
	cCmd := C.cec_command{}
	cCmd.initiator = C.cec_logical_address(command.Initiator)
	cCmd.destination = C.cec_logical_address(command.Destination)
	cCmd.opcode = C.cec_opcode(command.Opcode)
	cCmd.opcode_set = 1
	cCmd.parameters.size = C.uint8_t(len(command.Parameters))

	for i, param := range command.Parameters {
		cCmd.parameters.data[i] = C.uint8_t(param)
	}

	if C.libcec_transmit(c.handle, &cCmd) == 0 {
		return errors.New("failed to transmit command")
	}
	return nil
}

// SendKeypress sends a keypress
func (c *Connection) SendKeypress(address LogicalAddress, key Keycode, wait bool) error {
	waitVal := C.int(0)
	if wait {
		waitVal = 1
	}

	if C.libcec_send_keypress(c.handle, C.cec_logical_address(address),
		C.cec_user_control_code(key), waitVal) == 0 {
		return errors.New("failed to send keypress")
	}
	return nil
}

// SendKeyRelease sends a key release
func (c *Connection) SendKeyRelease(address LogicalAddress, wait bool) error {
	waitVal := C.int(0)
	if wait {
		waitVal = 1
	}

	if C.libcec_send_key_release(c.handle, C.cec_logical_address(address), waitVal) == 0 {
		return errors.New("failed to send key release")
	}
	return nil
}

// SetOSDString sets an OSD string
func (c *Connection) SetOSDString(address LogicalAddress, duration DisplayControl, message string) error {
	cMsg := C.CString(message)
	defer C.free(unsafe.Pointer(cMsg))

	if C.libcec_set_osd_string(c.handle, C.cec_logical_address(address),
		C.cec_display_control(duration), cMsg) == 0 {
		return errors.New("failed to set OSD string")
	}
	return nil
}

// SwitchMonitoring enables/disables monitoring mode
func (c *Connection) SwitchMonitoring(enable bool) error {
	val := C.int(0)
	if enable {
		val = 1
	}

	if C.libcec_switch_monitoring(c.handle, val) == 0 {
		return errors.New("failed to switch monitoring mode")
	}
	return nil
}

// GetLibInfo returns libcec version information
func (c *Connection) GetLibInfo() string {
	return C.GoString(C.libcec_get_lib_info(c.handle))
}

// SetConfiguration updates the configuration
func (c *Connection) SetConfiguration(config *Configuration) error {
	cConfig := C.libcec_configuration{}
	C.libcec_clear_configuration(&cConfig)

	cDeviceName := C.CString(config.DeviceName)
	defer C.free(unsafe.Pointer(cDeviceName))
	C.strncpy(&cConfig.strDeviceName[0], cDeviceName, 13)

	cConfig.deviceTypes.types[0] = C.cec_device_type(config.DeviceType)
	cConfig.iPhysicalAddress = C.uint16_t(config.PhysicalAddress)
	cConfig.baseDevice = C.cec_logical_address(config.BaseDevice)
	cConfig.iHDMIPort = C.uint8_t(config.HDMIPort)
	cConfig.clientVersion = C.uint32_t(config.ClientVersion)

	if C.libcec_set_configuration(c.handle, &cConfig) == 0 {
		return errors.New("failed to set configuration")
	}

	c.config = config
	return nil
}

// GetCurrentConfiguration retrieves the current configuration
func (c *Connection) GetCurrentConfiguration() (*Configuration, error) {
	var cConfig C.libcec_configuration
	if C.libcec_get_current_configuration(c.handle, &cConfig) == 0 {
		return nil, errors.New("failed to get current configuration")
	}

	config := &Configuration{
		DeviceName:      C.GoString(&cConfig.strDeviceName[0]),
		DeviceType:      DeviceType(cConfig.deviceTypes.types[0]),
		PhysicalAddress: uint16(cConfig.iPhysicalAddress),
		BaseDevice:      LogicalAddress(cConfig.baseDevice),
		HDMIPort:        uint8(cConfig.iHDMIPort),
		ClientVersion:   uint32(cConfig.clientVersion),
		ServerVersion:   uint32(cConfig.serverVersion),
	}

	return config, nil
}

// RescanDevices rescans for devices
func (c *Connection) RescanDevices() error {
	C.libcec_rescan_devices(c.handle)
	// Give devices time to respond
	time.Sleep(1 * time.Second)
	return nil
}

// GetLogicalAddresses returns all logical addresses currently assigned to this adapter.
// It inspects the cec_logical_addresses.addresses array and returns one entry
// per non-zero slot (0-15). If no entries are set but a primary address is
// known, it returns that single primary address.
func (c *Connection) GetLogicalAddresses() []LogicalAddress {
	addresses := C.libcec_get_logical_addresses(c.handle)

	var result []LogicalAddress
	for i := 0; i < 16; i++ {
		if addresses.addresses[i] != 0 {
			result = append(result, LogicalAddress(i))
		}
	}

	// Fallback: if no bit is set but a primary logical address is known, return it.
	if len(result) == 0 && addresses.primary != C.CECDEVICE_UNKNOWN {
		result = append(result, LogicalAddress(addresses.primary))
	}

	return result
}
