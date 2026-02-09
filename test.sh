#!/bin/bash

# Test script for CEC HTTP Bridge
# This script verifies that the service is working correctly

set -e

API_URL="${API_URL:-http://localhost:8080/api}"
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "========================================"
echo "CEC HTTP Bridge Test Suite"
echo "========================================"
echo ""
echo "Testing API at: $API_URL"
echo ""

# Test function
test_endpoint() {
    local name="$1"
    local method="$2"
    local endpoint="$3"
    local data="$4"
    
    echo -n "Testing $name... "
    
    if [ -z "$data" ]; then
        response=$(curl -s -w "\n%{http_code}" -X $method "$API_URL$endpoint" 2>/dev/null)
    else
        response=$(curl -s -w "\n%{http_code}" -X $method "$API_URL$endpoint" \
            -H "Content-Type: application/json" \
            -d "$data" 2>/dev/null)
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

# Counter for results
passed=0
failed=0

# Test health endpoint
echo "1. Basic Tests"
echo "   -----------"
if test_endpoint "Health check" "GET" "/health"; then
    ((passed++))
else
    ((failed++))
fi

# Test device listing
if test_endpoint "List devices" "GET" "/devices"; then
    ((passed++))
else
    ((failed++))
fi

# Test logs endpoint
if test_endpoint "Get logs" "GET" "/logs"; then
    ((passed++))
else
    ((failed++))
fi

echo ""
echo "2. Query Tests"
echo "   -----------"

# Test power status
if test_endpoint "Power status" "GET" "/power/status"; then
    ((passed++))
else
    ((failed++))
fi

# Test active source
if test_endpoint "Active source" "GET" "/source/active"; then
    ((passed++))
else
    ((failed++))
fi

echo ""
echo "3. Control Tests (WARNING: These will control your devices!)"
echo "   ---------------------------------------------------------"
read -p "Do you want to run control tests? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    # Test power on
    if test_endpoint "Power on TV" "POST" "/power/on"; then
        ((passed++))
    else
        ((failed++))
    fi
    
    sleep 1
    
    # Test power status after power on
    if test_endpoint "Power status after on" "GET" "/power/status"; then
        ((passed++))
    else
        ((failed++))
    fi
    
    # Test volume up
    if test_endpoint "Volume up" "POST" "/volume/up"; then
        ((passed++))
    else
        ((failed++))
    fi
    
    # Test volume down
    if test_endpoint "Volume down" "POST" "/volume/down"; then
        ((passed++))
    else
        ((failed++))
    fi
    
    # Test key press
    key_data='{"address": 4, "key": "up"}'
    if test_endpoint "Send key" "POST" "/key" "$key_data"; then
        ((passed++))
    else
        ((failed++))
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
echo ""

if [ $failed -eq 0 ]; then
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
