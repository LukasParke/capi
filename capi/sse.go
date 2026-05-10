package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"time"
)

// eventsSSEHandler streams CEC events to a client as Server-Sent Events.
//
// Each subscriber gets a buffered channel from the EventHub; non-blocking
// publishes mean a slow subscriber drops events (and bumps the dropped
// counter visible at /metrics) instead of stalling the publisher.
func eventsSSEHandler(w http.ResponseWriter, r *http.Request) {
	if eventHub == nil {
		respondError(w, http.StatusInternalServerError, "event hub not initialized")
		return
	}
	flusher, ok := w.(http.Flusher)
	if !ok {
		respondError(w, http.StatusInternalServerError, "streaming unsupported")
		return
	}

	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	w.Header().Set("X-Accel-Buffering", "no")
	w.WriteHeader(http.StatusOK)
	flusher.Flush()

	ch := eventHub.Subscribe()
	defer eventHub.Unsubscribe(ch)

	keepalive := time.NewTicker(15 * time.Second)
	defer keepalive.Stop()

	for {
		select {
		case ev, ok := <-ch:
			if !ok {
				return
			}
			body, err := json.Marshal(ev)
			if err != nil {
				continue
			}
			// Use named SSE event types so JS can addEventListener("power_change", ...).
			fmt.Fprintf(w, "event: %s\ndata: %s\n\n", ev.Type, body)
			flusher.Flush()
		case <-keepalive.C:
			fmt.Fprintf(w, ": keepalive\n\n")
			flusher.Flush()
		case <-r.Context().Done():
			return
		}
	}
}
