package main

import (
	"errors"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"time"
)

var (
	errStewardQueueFull   = errors.New("bus steward queue full")
	errStewardScanTimeout = errors.New("device scan timed out")
)

// devicesSourceCache, devicesSourceLight, devicesSourceFull are the labels
// returned alongside the device list to indicate where the data came from.
const (
	devicesSourceCache = "cache"
	devicesSourceLight = "wait_light"
	devicesSourceFull  = "wait_full"
	devicesSourceLive  = "live"
)

// devicesQuery captures the request shape for /api/devices.
type devicesQuery struct {
	live   bool
	rescan bool
	wait   time.Duration // explicit upper bound on synchronous wait
}

// parseDevicesQuery extracts ?live, ?rescan and ?wait parameters. Defaults
// when omitted: live=false, rescan=false, wait=0 (return cache).
//
// Replacing the previous unconditional 120s blocking wait with an opt-in
// `?wait=` (capped at maxDeviceWait) means the default request returns
// immediately even on a cold cache; clients still observe the refresh via
// the SSE devices_changed event.
func parseDevicesQuery(r *http.Request) (devicesQuery, error) {
	const maxDeviceWait = 10 * time.Second

	q := devicesQuery{}
	q.live = boolish(r.URL.Query().Get("live"))
	q.rescan = boolish(r.URL.Query().Get("rescan"))

	if w := strings.TrimSpace(r.URL.Query().Get("wait")); w != "" {
		// Accept either "5", "5s", or any duration string.
		if n, err := strconv.Atoi(w); err == nil {
			q.wait = time.Duration(n) * time.Second
		} else {
			d, err := time.ParseDuration(w)
			if err != nil {
				return q, fmt.Errorf("invalid wait %q: %w", w, err)
			}
			q.wait = d
		}
		if q.wait < 0 {
			return q, fmt.Errorf("wait must be >= 0")
		}
		if q.wait > maxDeviceWait {
			q.wait = maxDeviceWait
		}
	}
	if q.live || q.rescan {
		// Synchronous queries: wait at least the default if not explicitly set.
		if q.wait == 0 {
			q.wait = 5 * time.Second
		}
	}
	return q, nil
}

func boolish(s string) bool {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "1", "true", "yes", "on":
		return true
	}
	return false
}

// deviceListAfterSteward returns the current device list and a source label.
// It NEVER blocks longer than q.wait. The default (q.wait == 0) returns the
// cached snapshot immediately and triggers a background light refresh; the
// caller can then watch SSE devices_changed for follow-up updates.
func deviceListAfterSteward(q devicesQuery) ([]map[string]interface{}, string, error) {
	cache := func() []map[string]interface{} {
		snap := globalBusState.copySnapshot()
		out := make([]map[string]interface{}, len(snap.Devices))
		copy(out, snap.Devices)
		return out
	}

	if q.wait == 0 {
		// Async path: fire-and-forget background refresh, return cache.
		jobKind := stewardLight
		if !enqueueSteward(jobKind, nil) {
			// Queue full but we can still serve cache; not an error.
		}
		return cache(), devicesSourceCache, nil
	}

	// Synchronous path with bounded wait.
	jobKind := stewardLight
	if q.live || q.rescan {
		jobKind = stewardFull
	}
	done := make(chan struct{})
	if !enqueueSteward(jobKind, done) {
		return nil, "", errStewardQueueFull
	}
	timer := time.NewTimer(q.wait)
	defer timer.Stop()
	select {
	case <-done:
	case <-timer.C:
		return nil, "", errStewardScanTimeout
	}

	src := devicesSourceLight
	switch {
	case q.live:
		src = devicesSourceLive
	case q.rescan:
		src = devicesSourceFull
	}
	return cache(), src, nil
}

func getDevicesHandler(w http.ResponseWriter, r *http.Request) {
	if !requireCEC(w) {
		return
	}
	q, err := parseDevicesQuery(r)
	if err != nil {
		respondError(w, http.StatusBadRequest, err.Error())
		return
	}

	result, src, err := deviceListAfterSteward(q)
	if err != nil {
		switch err {
		case errStewardQueueFull:
			respondError(w, http.StatusServiceUnavailable, "Bus steward queue full; retry shortly")
		case errStewardScanTimeout:
			respondError(w, http.StatusGatewayTimeout, "Device scan timed out; partial result via /api/bus/state")
		default:
			respondError(w, http.StatusInternalServerError, err.Error())
		}
		return
	}

	w.Header().Set("X-Cache", src)
	appLog("devices", "GET /api/devices %s count=%d", src, len(result))
	msg := "Devices retrieved"
	if src == devicesSourceCache {
		msg = "Devices retrieved (cache)"
	} else {
		msg = fmt.Sprintf("Devices retrieved (%s)", src)
	}
	respondSuccess(w, msg, result)
}
