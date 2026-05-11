package main

import (
	"bufio"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"log"
	"net"
	"net/http"
	"runtime/debug"
	"strings"
	"sync/atomic"
	"time"
)

// HTTP request counters exposed at /metrics.
var (
	httpRequestsTotal   atomic.Uint64
	httpRequests4xx     atomic.Uint64
	httpRequests5xx     atomic.Uint64
	httpRequestsActive  atomic.Int64
	httpPanicsRecovered atomic.Uint64
)

// statusRecorder wraps http.ResponseWriter so middleware can observe the
// final status code written by the handler.
type statusRecorder struct {
	http.ResponseWriter
	status int
	bytes  int
}

func (r *statusRecorder) WriteHeader(code int) {
	r.status = code
	r.ResponseWriter.WriteHeader(code)
}

func (r *statusRecorder) Write(p []byte) (int, error) {
	if r.status == 0 {
		r.status = http.StatusOK
	}
	n, err := r.ResponseWriter.Write(p)
	r.bytes += n
	return n, err
}

// Flush forwards to the wrapped writer's Flusher impl so SSE handlers can
// flush each event.
func (r *statusRecorder) Flush() {
	if f, ok := r.ResponseWriter.(http.Flusher); ok {
		f.Flush()
	}
}

// Hijack forwards to the wrapped writer's Hijacker impl so the WebSocket
// upgrader (gorilla/websocket) can take over the underlying TCP connection.
// Without this, every request through loggingMiddleware fails the upgrade
// with "response does not implement http.Hijacker".
func (r *statusRecorder) Hijack() (net.Conn, *bufio.ReadWriter, error) {
	if h, ok := r.ResponseWriter.(http.Hijacker); ok {
		return h.Hijack()
	}
	return nil, nil, fmt.Errorf("statusRecorder: underlying writer is not a Hijacker")
}

func newRequestID() string {
	var b [8]byte
	if _, err := rand.Read(b[:]); err != nil {
		return fmt.Sprintf("rid-%d", time.Now().UnixNano())
	}
	return hex.EncodeToString(b[:])
}

// requestIDKey is the HTTP request header used to carry/return the request ID.
const requestIDHeader = "X-Request-ID"

// chain composes middlewares right-to-left so the first listed runs first.
func chain(h http.Handler, mws ...func(http.Handler) http.Handler) http.Handler {
	for i := len(mws) - 1; i >= 0; i-- {
		h = mws[i](h)
	}
	return h
}

// recoverMiddleware catches panics from downstream handlers, returns 500,
// logs the stack to journal, and bumps a counter.
func recoverMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer func() {
			if rec := recover(); rec != nil {
				httpPanicsRecovered.Add(1)
				appLog("http", "panic %s %s: %v\n%s", r.Method, r.URL.Path, rec, debug.Stack())
				if w.Header().Get("Content-Type") == "" {
					w.Header().Set("Content-Type", "application/json")
				}
				w.WriteHeader(http.StatusInternalServerError)
				_, _ = w.Write([]byte(`{"status":"error","message":"internal server error"}`))
			}
		}()
		next.ServeHTTP(w, r)
	})
}

// requestIDMiddleware ensures every request has an X-Request-ID. The ID is
// echoed back in the response header so clients can correlate logs.
func requestIDMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		rid := r.Header.Get(requestIDHeader)
		if rid == "" {
			rid = newRequestID()
			r.Header.Set(requestIDHeader, rid)
		}
		w.Header().Set(requestIDHeader, rid)
		next.ServeHTTP(w, r)
	})
}

// loggingMiddleware records counters and writes a structured access-log line.
// SSE/WebSocket paths are noisy by design; we suppress access logs for them
// but still count requests in the metrics.
func loggingMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()
		rec := &statusRecorder{ResponseWriter: w}
		httpRequestsTotal.Add(1)
		httpRequestsActive.Add(1)
		defer httpRequestsActive.Add(-1)

		next.ServeHTTP(rec, r)

		st := rec.status
		if st == 0 {
			st = http.StatusOK
		}
		switch {
		case st >= 500:
			httpRequests5xx.Add(1)
		case st >= 400:
			httpRequests4xx.Add(1)
		}

		// Suppress noisy access-log lines for streaming endpoints.
		if isStreamingPath(r.URL.Path) {
			return
		}
		appLog("http", "%s %s %d %dB %s rid=%s",
			r.Method, r.URL.Path, st, rec.bytes, time.Since(start).Truncate(time.Microsecond),
			r.Header.Get(requestIDHeader))
	})
}

func isStreamingPath(p string) bool {
	switch {
	case p == "/api/events":
		return true
	case strings.HasPrefix(p, "/api/events/ws"):
		return true
	case strings.HasPrefix(p, "/ui/static/"):
		return true
	}
	return false
}

// metricsHandler exposes a tiny Prometheus-style text endpoint. It is
// intentionally small (one file) rather than pulling in the official client
// library; everything reported here is already in the service's atomic
// counters.
func metricsHandler(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "text/plain; version=0.0.4")

	var b strings.Builder
	writeMetric := func(name, help string, kind string, value uint64, labels ...string) {
		fmt.Fprintf(&b, "# HELP %s %s\n", name, help)
		fmt.Fprintf(&b, "# TYPE %s %s\n", name, kind)
		if len(labels) == 0 {
			fmt.Fprintf(&b, "%s %d\n", name, value)
		} else {
			fmt.Fprintf(&b, "%s{%s} %d\n", name, strings.Join(labels, ","), value)
		}
	}
	writeMetricInt := func(name, help string, kind string, value int64) {
		fmt.Fprintf(&b, "# HELP %s %s\n", name, help)
		fmt.Fprintf(&b, "# TYPE %s %s\n", name, kind)
		fmt.Fprintf(&b, "%s %d\n", name, value)
	}

	writeMetric("capi_http_requests_total", "Total HTTP requests served", "counter", httpRequestsTotal.Load())
	writeMetric("capi_http_responses_4xx_total", "HTTP responses with 4xx status", "counter", httpRequests4xx.Load())
	writeMetric("capi_http_responses_5xx_total", "HTTP responses with 5xx status", "counter", httpRequests5xx.Load())
	writeMetric("capi_http_panics_recovered_total", "HTTP handler panics recovered by middleware", "counter", httpPanicsRecovered.Load())
	writeMetricInt("capi_http_requests_in_flight", "Currently in-flight HTTP requests", "gauge", httpRequestsActive.Load())

	if eventHub != nil {
		dropped, delivered := eventHub.Stats()
		writeMetric("capi_event_subscribers", "Active SSE/WebSocket subscribers", "gauge", uint64(eventHub.Subscribers()))
		writeMetric("capi_events_delivered_total", "Events successfully posted to a subscriber", "counter", delivered)
		writeMetric("capi_events_dropped_total", "Events dropped due to slow subscriber", "counter", dropped)
	}
	writeMetric("capi_steward_jobs_queued_total", "Steward jobs successfully enqueued", "counter", stewardJobsQueued.Load())
	writeMetric("capi_steward_jobs_dropped_total", "Steward jobs dropped because the queue was full", "counter", stewardJobsDropped.Load())

	if adapterReady() {
		writeMetric("capi_cec_adapter_ready", "1 if CEC adapter is attached, 0 otherwise", "gauge", 1)
	} else {
		writeMetric("capi_cec_adapter_ready", "1 if CEC adapter is attached, 0 otherwise", "gauge", 0)
	}

	_, _ = w.Write([]byte(b.String()))
}

// suppress "log" import being unused if logging middleware ever removes it
var _ = log.Println
