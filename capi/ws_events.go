package main

import (
	"encoding/json"
	"html"
	"log"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

var eventWSUpgrader = websocket.Upgrader{
	HandshakeTimeout: 10 * time.Second,
	ReadBufferSize:   1024,
	WriteBufferSize:  8192,
	CheckOrigin:      checkEventWSOrigin,
}

func checkEventWSOrigin(r *http.Request) bool {
	origin := r.Header.Get("Origin")
	if origin == "" {
		return true
	}
	u, err := url.Parse(origin)
	if err != nil {
		return false
	}
	wantHost, _, err := net.SplitHostPort(r.Host)
	if err != nil {
		wantHost = r.Host
	}
	return strings.EqualFold(u.Hostname(), wantHost)
}

// wsCoalesceWindow is the time window over which incoming events are batched
// before re-rendering OOB fragments. A noisy CEC bus (especially under
// monitoring=true) can fire many events in quick succession; rendering each
// one separately previously meant up to 4 template executions per event.
// Coalescing collapses bursts into a single render at most every window.
const wsCoalesceWindow = 120 * time.Millisecond

// eventsWebSocketHandler streams CEC events as HTML fragments for htmx 1.9
// hx-ws (OOB swaps). Live events are written to a per-connection feed; panel
// re-renders are coalesced over a short window so a noisy bus does not
// translate into a CPU storm of template executions.
func eventsWebSocketHandler(w http.ResponseWriter, r *http.Request) {
	if eventHub == nil {
		respondError(w, http.StatusInternalServerError, "event hub not initialized")
		return
	}

	conn, err := eventWSUpgrader.Upgrade(w, r, nil)
	if err != nil {
		log.Printf("[ws] upgrade: %v", err)
		return
	}
	defer conn.Close()

	ch := eventHub.Subscribe()
	defer eventHub.Unsubscribe(ch)

	done := make(chan struct{})
	go wsReadPump(conn, done)

	var writeMu sync.Mutex
	writeText := func(s string) error {
		writeMu.Lock()
		defer writeMu.Unlock()
		_ = conn.SetWriteDeadline(time.Now().Add(15 * time.Second))
		return conn.WriteMessage(websocket.TextMessage, []byte(s))
	}

	pingTicker := time.NewTicker(45 * time.Second)
	defer pingTicker.Stop()

	// coalescer holds a "what panels are dirty" set that accumulates between
	// renders. We render at most once per wsCoalesceWindow.
	var (
		flushTimer    *time.Timer
		flushTimerCh  <-chan time.Time
		dirtyDevices  bool
		dirtyTopology bool
		dirtySource   bool
	)

	scheduleFlush := func() {
		if flushTimer != nil {
			return
		}
		flushTimer = time.NewTimer(wsCoalesceWindow)
		flushTimerCh = flushTimer.C
	}

	flush := func() {
		flushTimer = nil
		flushTimerCh = nil
		ready := cecAdapterReady()
		if !ready {
			dirtyDevices, dirtyTopology, dirtySource = false, false, false
			return
		}
		var b strings.Builder
		bannerInner, err := executeTemplateString("bus_banner", busBannerTemplateData())
		if err == nil {
			b.WriteString(`<div id="bus-banner" hx-swap-oob="innerHTML">`)
			b.WriteString(bannerInner)
			b.WriteString(`</div>`)
		}
		if dirtyDevices {
			rows, msg := deviceRowsFromCurrentSnapshot()
			devInner, err := executeTemplateString("devices", map[string]interface{}{
				"Devices": rows,
				"Message": msg,
			})
			if err == nil {
				b.WriteString(`<div id="devices-panel" hx-swap-oob="innerHTML">`)
				b.WriteString(devInner)
				b.WriteString(`</div>`)
			}
		}
		if dirtySource {
			srcInner, err := executeTemplateString("source_panel", sourcePanelTemplateData())
			if err == nil {
				b.WriteString(`<div id="source-card" hx-swap-oob="innerHTML">`)
				b.WriteString(srcInner)
				b.WriteString(`</div>`)
			}
		}
		if dirtyTopology {
			topo := buildTopologyHDMIFragmentData()
			topoInner, err := executeTemplateString("topology_hdmi", topo)
			if err == nil {
				b.WriteString(`<div id="topology-card" hx-swap-oob="innerHTML">`)
				b.WriteString(topoInner)
				b.WriteString(`</div>`)
			}
		}
		dirtyDevices, dirtyTopology, dirtySource = false, false, false
		if b.Len() > 0 {
			_ = writeText(b.String())
		}
	}

	for {
		select {
		case <-r.Context().Done():
			return
		case <-done:
			return
		case <-pingTicker.C:
			writeMu.Lock()
			_ = conn.SetWriteDeadline(time.Now().Add(5 * time.Second))
			err := conn.WriteControl(websocket.PingMessage, nil, time.Now().Add(5*time.Second))
			writeMu.Unlock()
			if err != nil {
				return
			}
		case <-flushTimerCh:
			flush()
		case ev, ok := <-ch:
			if !ok {
				return
			}
			// Always send the live feed line immediately so timestamps in
			// the feed UI stay accurate.
			if line, err := buildEventFeedLine(ev); err == nil {
				if err := writeText(line); err != nil {
					return
				}
			}
			if shouldRefreshDevicesPanel(ev.Type) {
				dirtyDevices = true
			}
			if shouldRefreshTopology(ev.Type) {
				dirtyTopology = true
			}
			if ev.Type == "source_activated" {
				dirtySource = true
			}
			scheduleFlush()
		}
	}
}

func wsReadPump(conn *websocket.Conn, done chan<- struct{}) {
	defer close(done)
	conn.SetReadLimit(1 << 16)
	_ = conn.SetReadDeadline(time.Now().Add(120 * time.Second))
	conn.SetPongHandler(func(string) error {
		return conn.SetReadDeadline(time.Now().Add(120 * time.Second))
	})
	for {
		if _, _, err := conn.ReadMessage(); err != nil {
			return
		}
	}
}

// buildEventFeedLine returns just the per-event live-feed line, leaving panel
// re-renders to the coalescer.
func buildEventFeedLine(ev CECEvent) (string, error) {
	raw, err := json.Marshal(ev)
	if err != nil {
		return "", err
	}
	line := html.EscapeString(string(raw))
	var b strings.Builder
	b.WriteString(`<div id="ws-live-feed" hx-swap-oob="beforeend"><span class="ws-line">`)
	b.WriteString(line)
	b.WriteString("</span>\n</div>")
	return b.String(), nil
}

func shouldRefreshDevicesPanel(typ string) bool {
	switch typ {
	case "devices_changed", "configuration_changed", "adapter_state", "source_activated", "power_change":
		return true
	}
	return false
}

func shouldRefreshTopology(typ string) bool {
	switch typ {
	case "devices_changed", "configuration_changed", "adapter_state", "source_activated":
		return true
	}
	return false
}
