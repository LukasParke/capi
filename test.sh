#!/usr/bin/env bash

# Test script for CEC HTTP Bridge
# This script verifies that the service is working correctly
#
# Env overrides:
#   API_URL     base API URL (default http://localhost:8080/api)
#   CAPI_TOKEN  bearer token; when set, an auth-rejection suite runs
#               (401 without the key, 200 with it)

set -euo pipefail

API_URL="${API_URL:-http://localhost:8080/api}"
BASE_URL="${API_URL%/api}"
CAPI_TOKEN="${CAPI_TOKEN:-${API_TOKEN:-}}"
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "========================================"
echo "CEC HTTP Bridge Test Suite"
echo "========================================"
echo ""
echo "Testing API at: $API_URL"
if [[ -n "$CAPI_TOKEN" ]]; then
  echo "Auth token configured: running auth-rejection checks too."
fi
echo ""

# Test function. Extra curl args may follow endpoint/data.
test_endpoint() {
    local name="$1"
    local method="$2"
    local endpoint="$3"
    local data="${4:-}"
    shift $#
    local -a extra=("$@") curl_args=(-s --connect-timeout 5 --max-time 15 -w "\n%{http_code}")

    if [[ -n "$CAPI_TOKEN" ]]; then
        curl_args+=(-H "Authorization: Bearer ${CAPI_TOKEN}")
    fi

    echo -n "Testing $name... "

    local response http_code body
    if [ -z "$data" ]; then
        response=$(curl "${curl_args[@]}" -X "$method" "$API_URL$endpoint" "${extra[@]}" 2>/dev/null) || {
            echo -e "${RED}✗ FAIL (curl error)${NC}"
            return 1
        }
    else
        response=$(curl "${curl_args[@]}" -X "$method" "$API_URL$endpoint" \
            -H "Content-Type: application/json" \
            -d "$data" "${extra[@]}" 2>/dev/null) || {
            echo -e "${RED}✗ FAIL (curl error)${NC}"
            return 1
        }
    fi

    http_code=$(echo "$response" | tail -n 1)
    body=$(echo "$response" | head -n -1)

    if [ "$http_code" = "200" ]; then
        echo -e "${GREEN}✓ PASS${NC}"
        return 0
    else
        echo -e "${RED}✗ FAIL (HTTP $http_code)${NC}"
        echo "Response: $body"
        return 1
    fi
}

# Counter for results.
# NOTE: `((x++))` is an errexit trap under `set -e` — the post-increment
# expression evaluates to 0 (false) for x=0, aborting the whole script after
# the first passing test. Always use plain assignment instead.
passed=0
failed=0
skipped=0
pass() { passed=$((passed + 1)); }
fail_test() { failed=$((failed + 1)); }

echo ""
echo "1. Basic Tests"
echo "   -----------"

# Test health endpoint
if test_endpoint "Health check" "GET" "/health"; then
    pass
else
    fail_test
fi

# Test metrics endpoint (Prometheus text format, text/plain)
echo -n "Testing Metrics content-type... "
metrics_headers=$(curl -s -o /dev/null -D - \
    --connect-timeout 5 --max-time 15 "${BASE_URL}/metrics" 2>/dev/null || true)
if echo "$metrics_headers" | grep -qi '^content-type: *text/plain'; then
    echo -e "${GREEN}✓ PASS${NC}"
    pass
else
    echo -e "${RED}✗ FAIL (missing text/plain content-type)${NC}"
    fail_test
fi

# Adapter-dependent checks degrade to SKIP when no CEC adapter is present,
# so the suite is useful both on a Pi with hardware and in CI/dev without.
skip() { echo -e "${YELLOW}⊘ SKIP${NC}"; skipped=$((skipped+1)); }
ADAPTER_READY=0
health_body="$(curl -s --connect-timeout 5 --max-time 15 "$API_URL/health" 2>/dev/null || true)"
echo "$health_body" | grep -q '"cec_ready": *true' && ADAPTER_READY=1

# Test device listing
if [[ "$ADAPTER_READY" == "1" ]]; then
    if test_endpoint "List devices" "GET" "/devices"; then pass; else fail_test; fi
else
    echo -n "Testing List devices... "; skip
fi

# Devices cache shape: envelope data must carry per-device fields the UI and
# Home Assistant integrations rely on (logical address + discovery provenance).
if [[ -n "$CAPI_TOKEN" ]]; then
    curl -s --connect-timeout 5 --max-time 30 \
        -H "Authorization: Bearer ${CAPI_TOKEN}" \
        "$API_URL/devices" 2>/dev/null > /tmp/capi-devices.$$ || true
else
    curl -s --connect-timeout 5 --max-time 30 \
        "$API_URL/devices" 2>/dev/null > /tmp/capi-devices.$$ || true
fi
devices_body="$(cat /tmp/capi-devices.$$)"
rm -f /tmp/capi-devices.$$
echo -n "Testing Devices cache shape... "
if [[ "$ADAPTER_READY" != "1" ]]; then
    skip
elif echo "$devices_body" | grep -q '"status": *"success"' \
   && echo "$devices_body" | grep -q '"logical_address"' \
   && echo "$devices_body" | grep -q '"discovery"'; then
    echo -e "${GREEN}✓ PASS${NC}"
    pass
else
    echo -e "${RED}✗ FAIL (envelope/device fields missing)${NC}"
    fail_test
fi

# SSE stream must advertise text/event-stream.
echo -n "Testing SSE headers (/api/events)... "
auth_args=()
if [[ -n "$CAPI_TOKEN" ]]; then
    auth_args=(-H "Authorization: Bearer ${CAPI_TOKEN}")
fi
sse_headers=$(curl -s -N -o /dev/null -D - \
    --connect-timeout 5 --max-time 3 "${auth_args[@]}" \
    "$API_URL/events" 2>/dev/null || true)
if echo "$sse_headers" | grep -qi '^content-type: *text/event-stream'; then
    echo -e "${GREEN}✓ PASS${NC}"
    pass
else
    echo -e "${RED}✗ FAIL (expected text/event-stream)${NC}"
    fail_test
fi

# Auth-rejection checks: only meaningful (and only required) when a token is
# configured server-side. Without a token these would just 200 both ways.
if [[ -n "$CAPI_TOKEN" ]]; then
    echo ""
    echo "   Auth-rejection checks (token configured)"
    echo "   ----------------------------------------"

    echo -n "Testing request WITHOUT token is rejected (401)... "
    anon_code=$(curl -s -o /dev/null -w "%{http_code}" \
        --connect-timeout 5 --max-time 15 "$API_URL/devices" 2>/dev/null || true)
    if [ "$anon_code" = "401" ]; then
        echo -e "${GREEN}✓ PASS${NC}"
        pass
    else
        echo -e "${RED}✗ FAIL (expected 401, got HTTP $anon_code)${NC}"
        fail_test
    fi

    echo -n "Testing request WITH token succeeds (200)... "
    auth_code=$(curl -s -o /dev/null -w "%{http_code}" \
        --connect-timeout 5 --max-time 15 \
        -H "Authorization: Bearer ${CAPI_TOKEN}" "$API_URL/devices" 2>/dev/null || true)
    if [ "$auth_code" = "200" ]; then
        echo -e "${GREEN}✓ PASS${NC}"
        pass
    else
        echo -e "${RED}✗ FAIL (expected 200, got HTTP $auth_code)${NC}"
        fail_test
    fi
fi

# Test logs endpoint
if test_endpoint "Get logs" "GET" "/logs"; then
    pass
else
    fail_test
fi

echo ""
echo "2. Query Tests"
echo "   -----------"

# Test power status
if [[ "$ADAPTER_READY" != "1" ]]; then
    echo -n "Testing Power status... "; skip
elif test_endpoint "Power status" "GET" "/power/status"; then
    pass
else
    fail_test
fi

# Test active source
if [[ "$ADAPTER_READY" != "1" ]]; then
    echo -n "Testing Active source... "; skip
elif test_endpoint "Active source" "GET" "/source/active"; then
    pass
else
    fail_test
fi

echo ""
echo "3. Control Tests (WARNING: These will control your devices!)"
echo "   ---------------------------------------------------------"
if [[ ! -t 0 ]]; then
    echo "Non-interactive session — skipping control tests."
    REPLY="n"
else
    read -p "Do you want to run control tests? (y/N) " -n 1 -r
fi
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    # Test power on
    if test_endpoint "Power on TV" "POST" "/power/on"; then
        pass
    else
        fail_test
    fi

    sleep 1

    # Test power status after power on
    if test_endpoint "Power status after on" "GET" "/power/status"; then
        pass
    else
        fail_test
    fi

    # Test volume up
    if test_endpoint "Volume up" "POST" "/volume/up"; then
        pass
    else
        fail_test
    fi

    # Test volume down
    if test_endpoint "Volume down" "POST" "/volume/down"; then
        pass
    else
        fail_test
    fi

    # Test key press
    key_data='{"address": 4, "key": "select"}'
    if test_endpoint "Send key" "POST" "/key" "$key_data"; then
        pass
    else
        fail_test
    fi
else
    echo "Skipping control tests"
fi

echo ""
echo "========================================"
echo "Test Results"
echo "========================================"
echo -e "Passed: ${GREEN}$passed${NC}"
echo -e "Failed: ${RED}$failed${NC}"
echo -e "Skipped: ${YELLOW}$skipped${NC}"
echo ""

if [ "$failed" -eq 0 ]; then
    echo -e "${GREEN}All tests passed!${NC}"
    exit 0
else
    echo -e "${RED}Some tests failed.${NC}"
    echo ""
    echo "Troubleshooting:"
    echo "1. Check if service is running:"
    echo "   sudo systemctl status capi"
    echo ""
    echo "2. Check logs:"
    echo "   sudo journalctl -u capi -n 50"
    echo ""
    echo "3. Test CEC adapter:"
    echo "   cec-client -l"
    echo ""
    exit 1
fi
