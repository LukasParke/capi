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

LATEST=$(echo "$RELEASE_JSON" | grep "browser_download_url.*$BINARY" | head -1 | cut -d '"' -f 4)

if [ -z "$LATEST" ]; then
  echo "ERROR: Could not find download URL for $BINARY in latest release."
  echo "Check https://github.com/$REPO/releases"
  exit 1
fi

echo "Download URL: $LATEST"

# ── Install runtime dependencies ──────────────────────────────────────
echo "Installing runtime dependencies..."
apt-get update && apt-get install -y libcec6 cec-utils

# ── Download binary ───────────────────────────────────────────────────
mkdir -p /opt/capi
echo "Downloading $BINARY..."
curl -sSL --connect-timeout 10 --max-time 120 "$LATEST" -o /opt/capi/capi
chmod +x /opt/capi/capi

# ── Download support files from repo ──────────────────────────────────
BASE="https://raw.githubusercontent.com/$REPO/main"
echo "Downloading support files..."
curl -sSL --connect-timeout 10 --max-time 30 "$BASE/capi.service" -o /etc/systemd/system/capi.service
curl -sSL --connect-timeout 10 --max-time 30 "$BASE/99-cec.rules" -o /etc/udev/rules.d/99-cec.rules
curl -sSL --connect-timeout 10 --max-time 30 "$BASE/capi/index.html" -o /opt/capi/index.html

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
