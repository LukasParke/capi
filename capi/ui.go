package main

import (
	"bytes"
	"fmt"
	"html/template"
	"io/fs"
	"log"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/LukasParke/capi/cec"

	"github.com/gorilla/mux"
)

var uiTmpl *template.Template

func init() {
	uiTmpl = template.Must(template.New("").Funcs(uiFuncs()).ParseFS(uiTemplatesFS, "templates/*.gohtml"))
}

// uiFuncs is the template FuncMap used by every page. Kept tiny on purpose;
// presentation logic that needs more than a one-liner belongs in the Go
// handler that builds the data map.
func uiFuncs() template.FuncMap {
	return template.FuncMap{
		"iterRange": func(n int) []int {
			out := make([]int, n)
			for i := 0; i < n; i++ {
				out[i] = i
			}
			return out
		},
		"add": func(a, b int) int { return a + b },
	}
}

func cecAdapterReady() bool { return adapterReady() }

func registerUIHandlers(r *mux.Router) {
	staticFS, err := fs.Sub(uiStaticFS, "static")
	if err != nil {
		log.Fatalf("ui static embed: %v", err)
	}
	r.PathPrefix("/ui/static/").Handler(http.StripPrefix("/ui/static/", http.FileServer(http.FS(staticFS))))

	r.HandleFunc("/", uiLayoutHandler).Methods("GET")
	r.HandleFunc("/settings", uiSettingsLayoutHandler).Methods("GET")
	r.HandleFunc("/ui/fragment/bus_banner", uiFragmentBusBanner).Methods("GET")
	r.HandleFunc("/ui/fragment/devices", uiFragmentDevices).Methods("GET")
	r.HandleFunc("/ui/fragment/device_power", uiFragmentDevicePower).Methods("GET")
	r.HandleFunc("/ui/fragment/mqtt_panel", uiFragmentMQTTPanel).Methods("GET")
	r.HandleFunc("/ui/fragment/health", uiFragmentHealth).Methods("GET")
	// Legacy fragments retained for any external integrations that still
	// hit them; not referenced by the simplified remote layout.
	r.HandleFunc("/ui/fragment/topology_hdmi", uiFragmentTopologyHDMI).Methods("GET")
	r.HandleFunc("/ui/fragment/volume_panel", uiFragmentVolumePanel).Methods("GET")
	r.HandleFunc("/ui/fragment/nav_panel", uiFragmentNavPanel).Methods("GET")
	r.HandleFunc("/ui/fragment/source_panel", uiFragmentSourcePanel).Methods("GET")
	r.HandleFunc("/ui/fragment/logs", uiFragmentLogs).Methods("GET")

	r.HandleFunc("/ui/action/deep_scan", uiActionDeepScan).Methods("POST")
	r.HandleFunc("/ui/action/power_on", uiActionPowerOn).Methods("POST")
	r.HandleFunc("/ui/action/power_off", uiActionPowerOff).Methods("POST")
	r.HandleFunc("/ui/action/volume_up", uiActionVolumeUp).Methods("POST")
	r.HandleFunc("/ui/action/volume_down", uiActionVolumeDown).Methods("POST")
	r.HandleFunc("/ui/action/volume_mute", uiActionVolumeMute).Methods("POST")
	r.HandleFunc("/ui/action/set_source", uiActionSetSource).Methods("POST")
	r.HandleFunc("/ui/action/hdmi", uiActionHDMI).Methods("POST")
	r.HandleFunc("/ui/action/nav_key", uiActionNavKey).Methods("POST")
	r.HandleFunc("/ui/action/mqtt_save", uiActionMQTTSave).Methods("POST")

	// /dev page + its UI fragments and HTMX actions.
	r.HandleFunc("/dev", devLayoutHandler).Methods("GET")
	r.HandleFunc("/ui/dev/fragment/banner", uiDevFragmentBanner).Methods("GET")
	r.HandleFunc("/ui/dev/fragment/devices", uiDevFragmentDevices).Methods("GET")
	r.HandleFunc("/ui/dev/fragment/trace", uiDevFragmentTrace).Methods("GET")
	r.HandleFunc("/ui/dev/action/mode", uiDevActionMode).Methods("POST")
	r.HandleFunc("/ui/dev/action/probe", uiDevActionProbe).Methods("POST")
	r.HandleFunc("/ui/dev/action/send_key", uiDevActionSendKey).Methods("POST")
	r.HandleFunc("/ui/dev/action/send_opcode", uiDevActionSendOpcode).Methods("POST")
	r.HandleFunc("/ui/dev/action/run_strategies", uiDevActionRunStrategies).Methods("POST")
	r.HandleFunc("/ui/dev/action/save_strategy", uiDevActionSaveStrategy).Methods("POST")
}

func writeHTMLFragment(w http.ResponseWriter, name string, data interface{}) {
	var buf bytes.Buffer
	if err := uiTmpl.ExecuteTemplate(&buf, name, data); err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	buf.WriteTo(w)
}

func executeTemplateString(name string, data interface{}) (string, error) {
	var buf bytes.Buffer
	if err := uiTmpl.ExecuteTemplate(&buf, name, data); err != nil {
		return "", err
	}
	return buf.String(), nil
}

func busBannerTemplateData() map[string]interface{} {
	ready := adapterReady()
	snap := globalBusState.copySnapshot()
	snap.CECReady = ready
	if !ready {
		snap.Stale = true
	}
	last := ""
	if snap.LastFullScanAt != nil {
		last = snap.LastFullScanAt.Local().Format(time.RFC3339)
	}
	as := snap.ActiveSource
	if as < 0 {
		as = -1
	}
	return map[string]interface{}{
		"CECReady":       snap.CECReady,
		"ScanInProgress": snap.ScanInProgress,
		"Stale":          snap.Stale,
		"LastFullScan":   last,
		"Monitoring":     snap.Monitoring,
		"ActiveSource":   as,
		"DeviceCount":    len(snap.Devices),
	}
}

// topologyOwnAddresses returns the logical addresses currently bound to this
// adapter, suitable for marking "own" devices in UI lists. Returns an empty
// set when the adapter is not attached.
func topologyOwnAddresses() map[int]struct{} {
	own := map[int]struct{}{}
	_ = adapter.With(func(c *cec.Connection) error {
		p := buildTopologyPayloadLocked(c)
		for _, a := range p.OwnAddresses {
			own[a] = struct{}{}
		}
		return nil
	})
	return own
}

func buildDeviceRowsFromMaps(devices []map[string]interface{}, own map[int]struct{}, activeLA int) []uiDeviceRow {
	rows := make([]uiDeviceRow, 0, len(devices))
	for _, dm := range devices {
		la := intFromMap(dm, "logical_address")
		if la < 0 {
			continue
		}
		_, isOwn := own[la]
		rows = append(rows, deviceRowFromMap(dm, la, isOwn, activeLA))
	}
	return rows
}

// deviceRowFromMap renders a device payload (whether from /api/devices or
// the steward snapshot) into a uiDeviceRow. Empty strings are normalized to
// blank so the template can render a muted placeholder.
func deviceRowFromMap(dm map[string]interface{}, la int, isOwn bool, activeLA int) uiDeviceRow {
	discovery := stringFromMap(dm, "discovery")
	powerStatus := stringFromMap(dm, "power_status")
	if powerStatus == "" || powerStatus == "Unknown" {
		// Prefer observed power if libcec hasn't probed yet (ghost device path).
		if obs := stringFromMap(dm, "observed_power_status"); obs != "" {
			powerStatus = strings.Title(obs)
		}
	}

	row := uiDeviceRow{
		LogicalAddress:  la,
		OSDName:         stringFromMap(dm, "osd_name"),
		AddressName:     stringFromMap(dm, "address_name"),
		PhysicalAddress: stringFromMap(dm, "physical_address"),
		PowerStatus:     powerStatus,
		PowerObservedAt: stringFromMap(dm, "observed_at"),
		IsOwn:           isOwn,
		IsActiveSource:  (activeLA >= 0 && la == activeLA) || boolFromMap(dm, "is_active_source"),

		Role:          stringFromMap(dm, "device_type"),
		HDMIPort:      intFromMap(dm, "hdmi_port"),
		VendorID:      stringFromMap(dm, "vendor_id"),
		VendorName:    stringFromMap(dm, "vendor_name"),
		VendorKnown:   boolFromMap(dm, "vendor_known"),
		CECVersion:    stringFromMap(dm, "cec_version"),
		Discovery:     discovery,
		FirstSeen:     stringFromMap(dm, "first_seen_at"),
		LastSeen:      stringFromMap(dm, "last_seen_at"),
		FeatureAbortOpcode: intFromMap(dm, "observed_last_feature_abort_opcode"),
		FeatureAbortReason: intFromMap(dm, "observed_last_feature_abort_reason"),
		ObservedAudioMuted: boolFromMap(dm, "observed_audio_muted"),
		ObservedAudioRaw:   intFromMap(dm, "observed_audio_volume_raw"),
	}

	row.IsAudioSystem = la == 5
	row.IsGhost = discovery == "observed"

	// Friendly display name: prefer OSD, then observed OSD fragment, then role.
	row.DisplayName = row.OSDName
	if row.DisplayName == "" {
		row.DisplayName = stringFromMap(dm, "observed_osd_name_fragment")
	}
	if row.DisplayName == "" {
		// Use the role as a placeholder so the card never shows a blank name.
		if row.Role != "" {
			row.DisplayName = row.Role
		} else {
			row.DisplayName = row.AddressName
		}
	}
	return row
}

// deviceRowsFromCurrentSnapshot builds device cards from the steward snapshot only (no steward wait).
func deviceRowsFromCurrentSnapshot() ([]uiDeviceRow, string) {
	snap := globalBusState.copySnapshot()
	own := topologyOwnAddresses()
	activeLA := snap.ActiveSource
	rows := buildDeviceRowsFromMaps(snap.Devices, own, activeLA)
	return rows, fmt.Sprintf("Live snapshot (%d devices)", len(rows))
}

// uiLayoutHandler builds the data for the simplified universal-remote view.
// Quick-control buttons are pre-rendered server-side from the current bus
// state but are designed to render even if the bus is empty (HDMI buttons,
// for example, fall back to a default 1..4 list).
func uiLayoutHandler(w http.ResponseWriter, r *http.Request) {
	writeHTMLFragment(w, "layout", buildRemoteData())
}

// uiSettingsLayoutHandler renders the dedicated /settings page (MQTT,
// adapter info, update). The remote is intentionally free of these now.
func uiSettingsLayoutHandler(w http.ResponseWriter, r *http.Request) {
	writeHTMLFragment(w, "settings_layout", map[string]interface{}{
		"Version": version,
	})
}

// remoteHDMIDefault is the minimum number of HDMI port buttons we always
// render, even when the bus topology hasn't reported any ports yet. Keeps
// the "easily switch HDMI inputs even if other bus devices are not
// enumerating properly" guarantee true.
const remoteHDMIDefault = 4

// remoteNavTarget is one entry in the nav-pad target dropdown.
type remoteNavTarget struct {
	LA       int
	Label    string
	Selected bool
}

// buildRemoteData assembles the data map consumed by the layout +
// quick_controls + remote_pad templates. Centralized so all remote
// fragments stay consistent.
func buildRemoteData() map[string]interface{} {
	snap := globalBusState.copySnapshot()

	// HDMI ports: render at least 1..remoteHDMIDefault. If the topology
	// has reported a higher port count, extend up to that count.
	hdmiCount := remoteHDMIDefault
	activeHDMI := 0
	if c := adapter.Conn(); c != nil {
		topo := buildBusTopology(c)
		if int(topo.KnownPortCount) > hdmiCount {
			hdmiCount = int(topo.KnownPortCount)
		}
		// If active source's physical address tells us a port, mark it.
		if topo.OwnPort > 0 {
			activeHDMI = int(topo.OwnPort)
		}
	}
	hdmiPorts := make([]int, 0, hdmiCount)
	for i := 1; i <= hdmiCount; i++ {
		hdmiPorts = append(hdmiPorts, i)
	}

	// Nav-pad targets: every device on the bus, with the active source
	// pre-selected when known. If the bus is empty, list LA 0..4 so the
	// pad still has something to point at.
	navTargets := make([]remoteNavTarget, 0, len(snap.Devices)+5)
	seen := map[int]bool{}
	for _, dm := range snap.Devices {
		la, ok := dm["logical_address"].(int)
		if !ok || seen[la] {
			continue
		}
		seen[la] = true
		label := stringFromMap(dm, "address_name")
		if name := stringFromMap(dm, "osd_name"); name != "" {
			label = name
		}
		navTargets = append(navTargets, remoteNavTarget{
			LA:       la,
			Label:    label,
			Selected: snap.ActiveSource >= 0 && snap.ActiveSource == la,
		})
	}
	if len(navTargets) == 0 {
		for la := 0; la <= 4; la++ {
			navTargets = append(navTargets, remoteNavTarget{
				LA:       la,
				Label:    cec.LogicalAddress(la).String(),
				Selected: la == 0,
			})
		}
	}

	return map[string]interface{}{
		"Version":    version,
		"HDMIPorts":  hdmiPorts,
		"ActiveHDMI": activeHDMI,
		"NavTargets": navTargets,
	}
}

func uiFragmentBusBanner(w http.ResponseWriter, r *http.Request) {
	writeHTMLFragment(w, "bus_banner", busBannerTemplateData())
}

func uiFragmentDevices(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		w.WriteHeader(http.StatusServiceUnavailable)
		writeHTMLFragment(w, "devices", map[string]interface{}{
			"Devices": []uiDeviceRow{},
			"Message": "CEC adapter not available",
		})
		return
	}

	q, err := parseDevicesQuery(r)
	if err != nil {
		w.WriteHeader(http.StatusBadRequest)
		writeHTMLFragment(w, "devices", map[string]interface{}{
			"Devices": []uiDeviceRow{},
			"Message": err.Error(),
		})
		return
	}

	result, src, err := deviceListAfterSteward(q)
	if err != nil {
		msg := err.Error()
		switch err {
		case errStewardQueueFull:
			w.WriteHeader(http.StatusServiceUnavailable)
		case errStewardScanTimeout:
			w.WriteHeader(http.StatusGatewayTimeout)
		default:
			w.WriteHeader(http.StatusInternalServerError)
		}
		writeHTMLFragment(w, "devices", map[string]interface{}{
			"Devices": []uiDeviceRow{},
			"Message": msg,
		})
		return
	}

	own := topologyOwnAddresses()

	snap := globalBusState.copySnapshot()
	activeLA := snap.ActiveSource
	rows := buildDeviceRowsFromMaps(result, own, activeLA)

	writeHTMLFragment(w, "devices", map[string]interface{}{
		"Devices": rows,
		"Message": fmt.Sprintf("Source: %s (%d devices)", src, len(rows)),
	})
}

type uiDeviceRow struct {
	LogicalAddress  int
	DisplayName     string // best available human name (OSD -> observed -> role)
	OSDName         string
	AddressName     string
	PhysicalAddress string
	HDMIPort        int

	Role        string // device_type string (TV, Audio System, ...)
	VendorID    string // "0x000048"
	VendorName  string
	VendorKnown bool
	CECVersion  string

	PowerStatus     string
	PowerObservedAt string

	Discovery string // "active" | "polled" | "observed"
	IsGhost   bool   // discovery == "observed"
	FirstSeen string
	LastSeen  string

	FeatureAbortOpcode int
	FeatureAbortReason int

	IsAudioSystem      bool
	ObservedAudioMuted bool
	ObservedAudioRaw   int

	IsOwn          bool
	IsActiveSource bool
}

func uiFragmentDevicePower(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		writeHTMLFragment(w, "device_power", map[string]interface{}{"Error": "CEC adapter not available"})
		return
	}
	addr, err := strconv.Atoi(r.URL.Query().Get("addr"))
	if err != nil || addr < 0 || addr > 15 {
		writeHTMLFragment(w, "device_power", map[string]interface{}{"Error": "bad address"})
		return
	}
	st, err := execPowerStatus(addr)
	if err != nil {
		writeHTMLFragment(w, "device_power", map[string]interface{}{"Error": err.Error()})
		return
	}
	writeHTMLFragment(w, "device_power", map[string]interface{}{"Addr": addr, "Status": st})
}

func buildTopologyHDMIFragmentData() map[string]interface{} {
	c := adapter.Conn()
	if c == nil {
		return map[string]interface{}{"Ready": false}
	}
	p := buildTopologyPayloadLocked(c)

	maxPort := p.KnownPortCount
	if maxPort < 4 {
		maxPort = 4
	}
	ports := make([]int, 0, maxPort)
	for pn := 1; pn <= maxPort; pn++ {
		ports = append(ports, pn)
	}

	snap := globalBusState.copySnapshot()
	addrs := snap.LogicalAddresses
	if len(addrs) == 0 {
		for i := 0; i <= 4; i++ {
			addrs = append(addrs, i)
		}
	}

	return map[string]interface{}{
		"Ready":       true,
		"SelectAddrs": addrs,
		"PortNums":    ports,
	}
}

func uiFragmentTopologyHDMI(w http.ResponseWriter, r *http.Request) {
	writeHTMLFragment(w, "topology_hdmi", buildTopologyHDMIFragmentData())
}

func sourcePanelTemplateData() map[string]interface{} {
	c := adapter.Conn()
	if c == nil {
		return map[string]interface{}{"Ready": false}
	}
	var activeLA int
	var name string
	var qerr string
	addr, err := c.GetActiveSource()
	if err != nil {
		qerr = err.Error()
	} else {
		activeLA = int(addr)
		name = addr.String()
	}
	return map[string]interface{}{
		"Ready":      true,
		"ActiveLA":   activeLA,
		"ActiveName": name,
		"QueryErr":   qerr,
	}
}

type volOption struct {
	Value string
	Label string
}

func uiFragmentVolumePanel(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		writeHTMLFragment(w, "volume_panel", map[string]interface{}{
			"AudioLine": "CEC unavailable",
			"VolOptions": []volOption{{Value: "", Label: "Default (best effort)"}},
		})
		return
	}
	displayVol, muted, _, _ := execAudioStatusDisplay()
	audio := fmt.Sprintf("Vol %d%%", displayVol)
	if muted {
		audio += " (muted)"
	}

	snap := globalBusState.copySnapshot()
	opts := []volOption{{Value: "", Label: "Default (best effort)"}}
	for _, la := range snap.LogicalAddresses {
		name := cec.LogicalAddress(la).String()
		opts = append(opts, volOption{
			Value: strconv.Itoa(la),
			Label: fmt.Sprintf("%d — %s", la, name),
		})
	}
	if len(opts) == 1 {
		for la := 0; la <= 14; la++ {
			opts = append(opts, volOption{
				Value: strconv.Itoa(la),
				Label: fmt.Sprintf("%d — %s", la, cec.LogicalAddress(la).String()),
			})
		}
	}

	writeHTMLFragment(w, "volume_panel", map[string]interface{}{
		"AudioLine":  audio,
		"VolOptions": opts,
	})
}

func uiFragmentNavPanel(w http.ResponseWriter, r *http.Request) {
	snap := globalBusState.copySnapshot()
	addrs := snap.LogicalAddresses
	if len(addrs) == 0 {
		for i := 0; i <= 4; i++ {
			addrs = append(addrs, i)
		}
	}
	writeHTMLFragment(w, "nav_panel", map[string]interface{}{"Addrs": addrs})
}

func uiFragmentSourcePanel(w http.ResponseWriter, r *http.Request) {
	writeHTMLFragment(w, "source_panel", sourcePanelTemplateData())
}

func uiFragmentMQTTPanel(w http.ResponseWriter, r *http.Request) {
	configMu.RLock()
	cfg := currentConfig.MQTT
	configMu.RUnlock()
	pass := ""
	if cfg.Pass != "" {
		pass = "***"
	}
	mqttMu.Lock()
	connected := mqttClient != nil && mqttClient.IsConnected()
	mqttMu.Unlock()
	writeHTMLFragment(w, "mqtt_panel", map[string]interface{}{
		"Broker":    cfg.Broker,
		"User":      cfg.User,
		"PassHint":  pass,
		"Prefix":    cfg.Prefix,
		"Connected": connected,
	})
}

func uiFragmentLogs(w http.ResponseWriter, r *http.Request) {
	logs := logHandler.GetRecentLogs()
	writeHTMLFragment(w, "logs", map[string]interface{}{"Logs": logs})
}

func uiFragmentHealth(w http.ResponseWriter, r *http.Request) {
	writeHTMLFragment(w, "health", map[string]interface{}{
		"Healthy":  true,
		"CECReady": adapterReady(),
	})
}

func uiActionDeepScan(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		uiActionFail(w, "CEC adapter not available")
		return
	}
	done := make(chan struct{})
	if !enqueueSteward(stewardDeep, done) {
		w.Header().Set("HX-Trigger", "refresh")
		writeHTMLFragment(w, "action_note", map[string]interface{}{"OK": false, "Text": "Bus steward queue full; retry shortly."})
		return
	}
	w.Header().Set("HX-Trigger", "refresh")
	writeHTMLFragment(w, "action_note", map[string]interface{}{"OK": true, "Text": "Deep scan queued — watch banner until scan completes."})
}

func uiActionPowerOn(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		uiActionFail(w, "CEC adapter not available")
		return
	}
	addr, err := parseAddrParam(r, 0)
	if err != nil {
		uiActionFail(w, err.Error())
		return
	}
	if err := execPowerOn(addr); err != nil {
		uiActionFail(w, err.Error())
		return
	}
	uiActionOK(w, fmt.Sprintf("Power on → LA %d", addr))
}

func uiActionPowerOff(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		uiActionFail(w, "CEC adapter not available")
		return
	}
	addr, err := parseAddrParam(r, 0)
	if err != nil {
		uiActionFail(w, err.Error())
		return
	}
	if err := execPowerOff(addr); err != nil {
		uiActionFail(w, err.Error())
		return
	}
	uiActionOK(w, fmt.Sprintf("Standby → LA %d", addr))
}

func uiActionVolumeUp(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		uiActionFail(w, "CEC adapter not available")
		return
	}
	_ = r.ParseForm()
	addrStr := strings.TrimSpace(firstFormOrQuery(r, "addr"))
	msg, err := execVolumeUp(addrStr)
	if err != nil {
		uiActionFail(w, err.Error())
		return
	}
	uiActionOK(w, msg)
}

func uiActionVolumeDown(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		uiActionFail(w, "CEC adapter not available")
		return
	}
	_ = r.ParseForm()
	addrStr := strings.TrimSpace(firstFormOrQuery(r, "addr"))
	msg, err := execVolumeDown(addrStr)
	if err != nil {
		uiActionFail(w, err.Error())
		return
	}
	uiActionOK(w, msg)
}

func uiActionVolumeMute(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		uiActionFail(w, "CEC adapter not available")
		return
	}
	_ = r.ParseForm()
	addrStr := strings.TrimSpace(firstFormOrQuery(r, "addr"))
	msg, err := execVolumeMute(addrStr)
	if err != nil {
		uiActionFail(w, err.Error())
		return
	}
	uiActionOK(w, msg)
}

func uiActionSetSource(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		uiActionFail(w, "CEC adapter not available")
		return
	}
	_ = r.ParseForm()
	addrStr := firstFormOrQuery(r, "addr")
	if addrStr == "" {
		addrStr = r.URL.Query().Get("addr")
	}
	addr, err := strconv.Atoi(strings.TrimSpace(addrStr))
	if err != nil || addr < 0 || addr > 15 {
		uiActionFail(w, "invalid logical address")
		return
	}
	if err := execSetActiveSource(addr); err != nil {
		uiActionFail(w, err.Error())
		return
	}
	uiActionOK(w, fmt.Sprintf("Active source → LA %d", addr))
}

func uiActionHDMI(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		uiActionFail(w, "CEC adapter not available")
		return
	}
	port, err := strconv.Atoi(r.URL.Query().Get("port"))
	if err != nil || port < 1 || port > 15 {
		uiActionFail(w, "invalid port")
		return
	}
	if err := execHDMIPort(port); err != nil {
		uiActionFail(w, err.Error())
		return
	}
	uiActionOK(w, fmt.Sprintf("HDMI port %d", port))
}

func uiActionNavKey(w http.ResponseWriter, r *http.Request) {
	if !cecAdapterReady() {
		uiActionFail(w, "CEC adapter not available")
		return
	}
	_ = r.ParseForm()
	addr, err := strconv.Atoi(strings.TrimSpace(r.FormValue("addr")))
	if err != nil || addr < 0 || addr > 15 {
		uiActionFail(w, "invalid address")
		return
	}
	key := strings.TrimSpace(r.FormValue("key"))
	if err := execSendKey(addr, key, 0); err != nil {
		uiActionFail(w, err.Error())
		return
	}
	uiActionOK(w, fmt.Sprintf("Key %q → LA %d", key, addr))
}

func uiActionMQTTSave(w http.ResponseWriter, r *http.Request) {
	_ = r.ParseForm()
	pass := r.FormValue("pass")
	if r.FormValue("clear_password") == "1" {
		pass = ""
	} else if pass == "" {
		pass = "***"
	}
	if err := applyMQTTSettings(MQTTConfig{
		Broker: strings.TrimSpace(r.FormValue("broker")),
		User:   strings.TrimSpace(r.FormValue("user")),
		Pass:   pass,
		Prefix: strings.TrimSpace(r.FormValue("prefix")),
	}); err != nil {
		uiActionFail(w, err.Error())
		return
	}
	uiActionOK(w, "MQTT settings saved")
}

func uiActionOK(w http.ResponseWriter, text string) {
	w.Header().Set("HX-Trigger", "refresh")
	writeHTMLFragment(w, "action_note", map[string]interface{}{"OK": true, "Text": text})
}

func uiActionFail(w http.ResponseWriter, text string) {
	writeHTMLFragment(w, "action_note", map[string]interface{}{"OK": false, "Text": text})
}

// parseAddrParam returns the value of the "addr" form/query parameter, or
// (def, nil) when the parameter is absent. Returns an error when the value is
// present but not a valid logical address (0..15) so callers can report 400
// rather than silently fall back to the default (which used to mean a bad
// "addr" on /ui/action/power_on quietly powered the TV).
func parseAddrParam(r *http.Request, def int) (int, error) {
	_ = r.ParseForm()
	s := strings.TrimSpace(r.URL.Query().Get("addr"))
	if s == "" {
		s = strings.TrimSpace(r.FormValue("addr"))
	}
	if s == "" {
		return def, nil
	}
	v, err := strconv.Atoi(s)
	if err != nil {
		return 0, fmt.Errorf("invalid address %q", s)
	}
	if v < 0 || v > 15 {
		return 0, fmt.Errorf("address %d out of range (0-15)", v)
	}
	return v, nil
}

func firstFormOrQuery(r *http.Request, key string) string {
	if v := r.FormValue(key); v != "" {
		return v
	}
	return r.URL.Query().Get(key)
}

func intFromMap(m map[string]interface{}, key string) int {
	v, ok := m[key]
	if !ok {
		return -1
	}
	switch x := v.(type) {
	case int:
		return x
	case int64:
		return int(x)
	case float64:
		return int(x)
	default:
		return -1
	}
}

func stringFromMap(m map[string]interface{}, key string) string {
	v, ok := m[key]
	if !ok || v == nil {
		return ""
	}
	switch x := v.(type) {
	case string:
		return x
	default:
		return fmt.Sprint(x)
	}
}

func boolFromMap(m map[string]interface{}, key string) bool {
	v, ok := m[key]
	if !ok {
		return false
	}
	b, ok := v.(bool)
	return ok && b
}
