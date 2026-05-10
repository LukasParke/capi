package main

import (
	"encoding/json"
	"net/http"
)

// Response is the standard JSON envelope returned by every /api/* handler.
//
//	{"status": "success"|"error", "message": "...", "data": ...}
type Response struct {
	Status  string      `json:"status"`
	Message string      `json:"message,omitempty"`
	Data    interface{} `json:"data,omitempty"`
}

// respondJSON writes the given value as JSON with the given status code.
func respondJSON(w http.ResponseWriter, status int, data interface{}) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(data)
}

// respondError writes a Response with status="error".
func respondError(w http.ResponseWriter, status int, message string) {
	respondJSON(w, status, Response{Status: "error", Message: message})
}

// respondSuccess writes a Response with status="success".
func respondSuccess(w http.ResponseWriter, message string, data interface{}) {
	respondJSON(w, http.StatusOK, Response{Status: "success", Message: message, Data: data})
}

// requireCEC returns true if the adapter is ready, otherwise it writes a 503
// JSON error and returns false. Callers can `if !requireCEC(w) { return }`.
func requireCEC(w http.ResponseWriter) bool {
	if !adapterReady() {
		respondError(w, http.StatusServiceUnavailable, "CEC adapter not available")
		return false
	}
	return true
}
