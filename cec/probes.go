package cec

// Optional Give* / query transmits used by bus stewards to refresh CEC state
// indirectly (devices respond on the bus and libcec updates its caches).
// Initiator is always the adapter's own logical address.

func (c *Connection) transmitSimple(initiator, dest LogicalAddress, op Opcode, params []uint8) error {
	return c.Transmit(&Command{
		Initiator:   initiator,
		Destination: dest,
		Opcode:      op,
		OpcodeSet:   true,
		Parameters:  params,
	})
}

// GivePhysicalAddressBroadcast broadcasts a request for devices to report
// their physical address.
func (c *Connection) GivePhysicalAddressBroadcast() error {
	return c.transmitSimple(c.ownAddress(), LogicalAddressBroadcast, OpcodeGivePhysicalAddress, nil)
}

// GiveDeviceVendorID requests vendor ID from dest.
func (c *Connection) GiveDeviceVendorID(dest LogicalAddress) error {
	return c.transmitSimple(c.ownAddress(), dest, OpcodeGiveDeviceVendorID, nil)
}

// GiveOSDName requests OSD name from dest.
func (c *Connection) GiveOSDName(dest LogicalAddress) error {
	return c.transmitSimple(c.ownAddress(), dest, OpcodeGiveOSDName, nil)
}

// GiveDevicePowerStatus requests power status from dest.
func (c *Connection) GiveDevicePowerStatus(dest LogicalAddress) error {
	return c.transmitSimple(c.ownAddress(), dest, OpcodeGiveDevicePowerStatus, nil)
}

// GiveAudioStatus requests audio status from dest (typically audio system).
func (c *Connection) GiveAudioStatus(dest LogicalAddress) error {
	return c.transmitSimple(c.ownAddress(), dest, OpcodeGiveAudioStatus, nil)
}

// GiveSystemAudioModeStatus requests system audio mode status from dest.
func (c *Connection) GiveSystemAudioModeStatus(dest LogicalAddress) error {
	return c.transmitSimple(c.ownAddress(), dest, OpcodeGiveSystemAudioModeStatus, nil)
}

// GiveDeckStatus requests deck status (playback devices).
// mode: 1 = on, 2 = off, 3 = once.
func (c *Connection) GiveDeckStatus(dest LogicalAddress, mode uint8) error {
	return c.transmitSimple(c.ownAddress(), dest, OpcodeGiveDeckStatus, []uint8{mode})
}

// GiveTunerDeviceStatus requests tuner status.
func (c *Connection) GiveTunerDeviceStatus(dest LogicalAddress, param uint8) error {
	return c.transmitSimple(c.ownAddress(), dest, OpcodeGiveTunerDeviceStatus, []uint8{param})
}

// GiveMenuLanguage requests menu language from dest.
func (c *Connection) GiveMenuLanguage(dest LogicalAddress) error {
	return c.transmitSimple(c.ownAddress(), dest, OpcodeGetMenuLanguage, nil)
}

// MenuRequest sends Menu Request (query: 0 = activate, 1 = deactivate, 2 = query).
func (c *Connection) MenuRequest(dest LogicalAddress, query uint8) error {
	return c.transmitSimple(c.ownAddress(), dest, OpcodeMenuRequest, []uint8{query})
}
