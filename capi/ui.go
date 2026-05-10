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
	uiTmpl = template.Must(template.ParseFS(uiTemplatesFS, "templates/*.gohtml"))
}

func cecAdapterReady() bool { return adapterReady() }

func registerUIHandlers(r *mux.Router) {
	staticFS, err := fs.Sub(uiStaticFS, "static")
	if err != nil {
		log.Fatalf("ui static embed: %v", err)
	}
	r.PathPrefix("/ui/static/").Handler(http.StripPrefix("/ui/static/", http.FileServer(http.FS(staticFS))))

	r.HandleFunc("/", uiLayoutHandler).Methods("GET")
	r.HandleFunc("/ui/fragment/bus_banner", uiFragmentBusBanner).Methods("GET")
	r.HandleFunc("/ui/fragment/devices", uiFragmentDevices).Methods("GET")
	r.HandleFunc("/ui/fragment/device_power", uiFragmentDevicePower).Methods("GET")
	r.HandleFunc("/ui/fragment/topology_hdmi", uiFragmentTopologyHDMI).Methods("GET")
	r.HandleFunc("/ui/fragment/volume_panel", uiFragmentVolumePanel).Methods("GET")
	r.HandleFunc("/ui/fragment/nav_panel", uiFragmentNavPanel).Methods("GET")
	r.HandleFunc("/ui/fragment/source_panel", uiFragmentSourcePanel).Methods("GET")
	r.HandleFunc("/ui/fragment/mqtt_panel", uiFragmentMQTTPanel).Methods("GET")
	r.HandleFunc("/ui/fragment/logs", uiFragmentLogs).Methods("GET")
	r.HandleFunc("/ui/fragment/health", uiFragmentHealth).Methods("GET")

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
		rows = append(rows, uiDeviceRow{
			LogicalAddress:  la,
			OSDName:         stringFromMap(dm, "osd_name"),
			AddressName:     stringFromMap(dm, "address_name"),
			PhysicalAddress: stringFromMap(dm, "physical_address"),
			PowerStatus:     stringFromMap(dm, "power_status"),
			IsOwn:           isOwn,
			IsActiveSource:  (activeLA >= 0 && la == activeLA) || boolFromMap(dm, "is_active_source"),
		})
	}
	return rows
}

// deviceRowsFromCurrentSnapshot builds device cards from the steward snapshot only (no steward wait).
func deviceRowsFromCurrentSnapshot() ([]uiDeviceRow, string) {
	snap := globalBusState.copySnapshot()
	own := topologyOwnAddresses()
	activeLA := snap.ActiveSource
	rows := buildDeviceRowsFromMaps(snap.Devices, own, activeLA)
	return rows, fmt.Sprintf("Live snapshot (%d devices)", len(rows))
}

func uiLayoutHandler(w http.ResponseWriter, r *http.Request) {
	writeHTMLFragment(w, "layout", map[string]interface{}{
		"Version": version,
	})
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
	LogicalAddress   int
	OSDName          string
	AddressName      string
	PhysicalAddress  string
	PowerStatus      string
	IsOwn            bool
	IsActiveSource   bool
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
