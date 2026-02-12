# Quick Start Guide

Build and deploy **directly on a Raspberry Pi** (or similar device with a CEC adapter). There is no cross-compilation: you run `make build` and `make deploy` on the Pi itself.

## Installation (5 minutes)

### 1. Install Dependencies

On the Raspberry Pi (Raspberry Pi OS, or Debian/Ubuntu):

```bash
# Option A: use make setup (recommended on Pi)
sudo make setup

# Option B: install manually
sudo apt-get update
sudo apt-get install -y libcec-dev libcec6 cec-utils pkg-config

# Go: on Raspberry Pi OS either use the package manager or official tarball
sudo apt-get install -y golang-go
# Or for a newer Go: https://go.dev/dl/ — use linux/arm64 (Pi 3/4/5) or linux/armv6l (Pi Zero/1)
```

### 2. Build the Project

On the Pi, in the project directory:

```bash
cd /path/to/capi
go mod tidy
make build
```

### 3. Test It Works

```bash
# Test CEC adapter is detected
cec-client -l

# Run the example
go run ./examples/example.go

# Or run the HTTP service
./capi
```

### 4. Install as Service (Optional)

Still on the Pi:

```bash
# One-command deploy (install, enable, and start)
sudo make deploy

# Or step by step:
sudo make install
sudo systemctl enable capi
sudo systemctl start capi

# Check it's running
sudo systemctl status capi
```

The service runs as the system user `capi` and listens on all interfaces (port 8080), so other devices on your network can reach it (e.g. `http://<pi-ip>:8080/api/health`). The `platform/` directory is optional when using the system `libcec-dev` package.

## First API Calls

Once the service is running, try these commands:

```bash
# Get all devices
curl http://localhost:8080/api/devices | jq

# Power on TV
curl -X POST http://localhost:8080/api/power/on

# Get power status
curl http://localhost:8080/api/power/status | jq

# Switch to HDMI 2
curl -X POST http://localhost:8080/api/hdmi/2

# Volume up
curl -X POST http://localhost:8080/api/volume/up

# Health check
curl http://localhost:8080/api/health | jq
```

## Common Issues

### "No CEC adapters found"

**Solution:**
```bash
# Check if device is detected
ls -la /dev/ttyACM*

# Add user to dialout group
sudo usermod -a -G dialout $USER
# Log out and back in

# For Raspberry Pi, enable CEC in config
echo "hdmi_ignore_cec=0" | sudo tee -a /boot/config.txt
sudo reboot
```

### "Failed to open adapter"

**Solution:**
```bash
# Ensure udev rules are loaded (make install installs 99-cec.rules for group capi)
sudo udevadm control --reload-rules
# Unplug and replug the USB CEC adapter so the rule applies to the device

# Temporary fix: relax permissions (not needed if udev rule is correct)
sudo chmod 666 /dev/ttyACM0
```

### "Service won't start" (exit-code / activating (auto-restart))

**Solution:**
```bash
# 1. See why it failed (most important)
sudo journalctl -u capi -n 80 --no-pager

# 2. Run as the service user to reproduce the error
sudo -u capi /opt/capi/capi -bind :8080 -name "CEC HTTP Bridge"

# 3. If "No CEC adapters found": plug in the USB CEC adapter, then sudo systemctl restart capi
# 4. If "Failed to open CEC adapter": check device exists and udev rules (see "Failed to open adapter" above)
# 5. Check if port is already in use
sudo lsof -i :8080
```

## Configuration

### Change Port

Edit `/etc/systemd/system/capi.service`:
```ini
ExecStart=/opt/capi/capi -bind :9090
```

Then reload:
```bash
sudo systemctl daemon-reload
sudo systemctl restart capi
```

### Local Only Access

Change bind address:
```ini
ExecStart=/opt/capi/capi -bind localhost:8080
```

### Custom Device Name

```ini
ExecStart=/opt/capi/capi -name "My Custom Name"
```

## Example Scripts

### Python Script

```python
#!/usr/bin/env python3
import requests
import time

BASE_URL = "http://localhost:8080/api"

def main():
    # Power on TV
    print("Powering on TV...")
    requests.post(f"{BASE_URL}/power/on")
    time.sleep(2)
    
    # Switch to HDMI 2
    print("Switching to HDMI 2...")
    requests.post(f"{BASE_URL}/hdmi/2")
    time.sleep(1)
    
    # Get devices
    print("Getting devices...")
    response = requests.get(f"{BASE_URL}/devices")
    devices = response.json()['data']
    
    for device in devices:
        print(f"  {device['address_name']}: {device['osd_name']}")
        print(f"    Power: {device['power_status']}")
        print(f"    Vendor: {device['vendor_name']}")
        print()

if __name__ == "__main__":
    main()
```

### Shell Script

```bash
#!/bin/bash
API="http://localhost:8080/api"

echo "Morning routine starting..."

# Power on TV
echo "Powering on TV..."
curl -s -X POST "$API/power/on"
sleep 2

# Switch to HDMI 1
echo "Switching to HDMI 1..."
curl -s -X POST "$API/hdmi/1"
sleep 1

# Set volume
echo "Setting volume..."
for i in {1..5}; do
    curl -s -X POST "$API/volume/up"
    sleep 0.2
done

echo "Done!"
```

### Node.js Script

```javascript
const axios = require('axios');

const BASE_URL = 'http://localhost:8080/api';

async function main() {
    try {
        // Power on TV
        console.log('Powering on TV...');
        await axios.post(`${BASE_URL}/power/on`);
        await sleep(2000);
        
        // Get devices
        console.log('Getting devices...');
        const response = await axios.get(`${BASE_URL}/devices`);
        const devices = response.data.data;
        
        devices.forEach(device => {
            console.log(`${device.address_name}: ${device.osd_name}`);
            console.log(`  Power: ${device.power_status}`);
            console.log(`  Vendor: ${device.vendor_name}`);
        });
        
    } catch (error) {
        console.error('Error:', error.message);
    }
}

function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

main();
```

## Next Steps

1. **Read the full API documentation** in README.md
2. **Explore the Go package** for custom integrations
3. **Set up automation** with your home automation system
4. **Check the examples** directory for more code samples

## Getting Help

- Check the README.md for full documentation
- Look at examples in the `examples/` directory
- Test with `cec-client` to verify hardware is working
- Check logs with `sudo journalctl -u capi -f`

## Uninstall

```bash
# Remove service
sudo make uninstall

# Remove source
cd ..
rm -rf capi
```
