package main

import (
	"sort"

	"github.com/LukasParke/capi/cec"
)

// PortInfo describes one HDMI port on the display and which devices are on it.
type PortInfo struct {
	Port    uint8                `json:"port"`
	Devices []cec.LogicalAddress `json:"devices"`
}

// BusTopology describes the HDMI bus as seen through CEC.
type BusTopology struct {
	OwnAddress     cec.LogicalAddress `json:"own_address"`
	OwnPort        uint8              `json:"own_port"`
	ActivePorts    []PortInfo         `json:"active_ports"`
	KnownPortCount uint8              `json:"known_port_count"`
}

// TopologyPortRow is one HDMI port row for UI / JSON.
type TopologyPortRow struct {
	Port    int
	Devices []string
}

// TopologyPayload is shared by GET /api/topology and HTMX fragments.
type TopologyPayload struct {
	OwnAddresses   []int
	OwnPort        int
	KnownPortCount int
	Ports          []TopologyPortRow
}

// buildBusTopology builds a topology of the CEC bus by inspecting the
// physical addresses of all visible devices and grouping them by HDMI port.
//
// The caller must already hold cecMutex (or otherwise serialize access to
// conn). All libcec calls inside conn are also internally serialized, but
// this function makes many of them - call it off the request goroutine.
func buildBusTopology(conn *cec.Connection) *BusTopology {
	topo := &BusTopology{}

	addrs := conn.GetLogicalAddresses()
	if len(addrs) > 0 {
		topo.OwnAddress = addrs[0]
	} else {
		topo.OwnAddress = cec.LogicalAddressFreeUse
	}

	if topo.OwnAddress.IsValid() {
		if phys, err := conn.GetDevicePhysicalAddress(topo.OwnAddress); err == nil && phys != 0 && phys != 0xFFFF {
			topo.OwnPort = uint8((phys >> 12) & 0xF)
		}
	}

	portMap := make(map[uint8][]cec.LogicalAddress)
	for _, addr := range conn.LogicalAddressesWithOptionalPoll(false) {
		if addr == cec.LogicalAddressTV {
			continue
		}
		phys, err := conn.GetDevicePhysicalAddress(addr)
		if err != nil || phys == 0 || phys == 0xFFFF {
			continue
		}
		port := uint8((phys >> 12) & 0xF)
		if port == 0 {
			continue
		}
		portMap[port] = append(portMap[port], addr)
		if port > topo.KnownPortCount {
			topo.KnownPortCount = port
		}
	}

	for p := uint8(1); p <= topo.KnownPortCount; p++ {
		if devs, ok := portMap[p]; ok {
			sort.Slice(devs, func(i, j int) bool { return devs[i] < devs[j] })
			topo.ActivePorts = append(topo.ActivePorts, PortInfo{Port: p, Devices: devs})
		}
	}

	return topo
}

// buildTopologyPayloadLocked is the UI-facing topology builder. Caller must
// hold cecMutex.
func buildTopologyPayloadLocked(conn *cec.Connection) *TopologyPayload {
	topo := buildBusTopology(conn)
	ownAddrs := conn.GetLogicalAddresses()

	ports := make([]TopologyPortRow, 0, len(topo.ActivePorts))
	for _, p := range topo.ActivePorts {
		names := make([]string, 0, len(p.Devices))
		for _, addr := range p.Devices {
			name, _ := conn.GetDeviceOSDName(addr)
			if name == "" {
				name = addr.String()
			}
			names = append(names, name)
		}
		ports = append(ports, TopologyPortRow{Port: int(p.Port), Devices: names})
	}

	ownInts := make([]int, len(ownAddrs))
	for i, a := range ownAddrs {
		ownInts[i] = int(a)
	}

	return &TopologyPayload{
		OwnAddresses:   ownInts,
		OwnPort:        int(topo.OwnPort),
		KnownPortCount: int(topo.KnownPortCount),
		Ports:          ports,
	}
}
