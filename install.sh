#!/bin/bash
set -e

# ── Detect architecture ───────────────────────────────────────────────
ARCH=$(uname -m)
case "$ARCH" in
  aarch64)       BINARY="capi-linux-arm64" ;;
  armv7l|armv6l) BINARY="capi-linux-armv6" ;;
  *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

echo "Detected architecture: $ARCH -> $BINARY"

# ── Get latest release download URL from GitHub API ───────────────────
REPO="LukasParke/capi"
echo "Fetching latest release info..."
RELEASE_JSON=$(curl -sSL --connect-timeout 10 --max-time 30 \
  "https://api.github.com/repos/$REPO/releases/latest")

# Helper: extract asset download URL by name from the release JSON.
asset_url() {
  echo "$RELEASE_JSON" | grep "browser_download_url.*/$1\"" | head -1 | cut -d '"' -f 4
}

# Helper: download a file with retries, HTTP error detection, and diagnostics.
download() {
  local url="$1" dest="$2" label="$3"
  echo "Downloading ${label}..."
  if ! curl -fsSL --retry 3 --retry-delay 2 --connect-timeout 10 --max-time 300 "$url" -o "$dest"; then
    echo ""
    echo "ERROR: Failed to download ${label}."
    echo "  URL:  $url"
    echo "  Dest: $dest"
    echo "  Disk: $(df -h /opt/capi | awk 'NR==2 {print $4}') available"
    echo ""
    echo "Check your network connection and available disk space."
    exit 1
  fi
}

BINARY_URL=$(asset_url "$BINARY")
if [ -z "$BINARY_URL" ]; then
  echo "ERROR: Could not find download URL for $BINARY in latest release."
  echo "Check https://github.com/$REPO/releases"
  exit 1
fi

echo "Binary URL: $BINARY_URL"

# ── Install runtime dependencies ──────────────────────────────────────
echo "Installing runtime dependencies..."
apt-get update && apt-get install -y libcec6 cec-utils && apt-get clean

# ── Stop existing service if running ──────────────────────────────────
if systemctl is-active --quiet capi.service 2>/dev/null; then
  echo "Stopping existing capi service..."
  systemctl stop capi.service
fi

# ── Download binary ───────────────────────────────────────────────────
mkdir -p /opt/capi

AVAIL_KB=$(df /opt/capi | awk 'NR==2 {print $4}')
if [ "$AVAIL_KB" -lt 51200 ]; then
  echo "ERROR: Not enough disk space on /opt (${AVAIL_KB} KB available, need ~50 MB)."
  echo "Free up space and try again."
  exit 1
fi

download "$BINARY_URL" /opt/capi/capi "$BINARY"
chmod +x /opt/capi/capi

# ── Download support files from release ───────────────────────────────
download "$(asset_url capi.service)" /etc/systemd/system/capi.service "capi.service"
download "$(asset_url 99-cec.rules)" /etc/udev/rules.d/99-cec.rules "99-cec.rules"
download "$(asset_url index.html)"   /opt/capi/index.html "index.html"

# ── Create service user ───────────────────────────────────────────────
id -u capi &>/dev/null || useradd --system --user-group --no-create-home --shell /usr/sbin/nologin capi

# ── Reload and start ──────────────────────────────────────────────────
systemctl daemon-reload
udevadm control --reload-rules
systemctl enable capi.service
systemctl restart capi.service

echo ""
echo "capi installed and running."
echo "Visit http://$(hostname -I | awk '{print $1}'):8080"
