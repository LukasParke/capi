# Quick Start Guide

## Installation (5 minutes)

### 1. Install Dependencies

```bash
# Update package list
sudo apt-get update

# Install libcec and development files
sudo apt-get install -y libcec-dev libcec6 cec-utils pkg-config

# Install Go (if not already installed)
wget https://go.dev/dl/go1.21.0.linux-arm64.tar.gz
sudo tar -C /usr/local -xzf go1.21.0.linux-arm64.tar.gz
export PATH=$PATH:/usr/local/go/bin
```

### 2. Build the Project

```bash
# Navigate to project directory
cd /path/to/capi

# Initialize Go modules
go mod init capi
go mod tidy

# Build
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

```bash
# Install and enable service
sudo make install
sudo systemctl enable capi
sudo systemctl start capi

# Check it's running
sudo systemctl status capi
```

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
# Check permissions
sudo chmod 666 /dev/ttyACM0

# Or add permanent udev rule
echo 'SUBSYSTEM=="tty", ATTRS{idVendor}=="2548", ATTRS{idProduct}=="1001", MODE="0666"' | sudo tee /etc/udev/rules.d/99-cec.rules
sudo udevadm control --reload-rules
```

### "Service won't start"

**Solution:**
```bash
# Check logs
sudo journalctl -u capi -n 50

# Try running manually to see errors
sudo -u pi /opt/capi/capi

# Check if port is already in use
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
