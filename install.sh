#!/usr/bin/env bash
# capi installer: download + verify + install the latest GitHub release.
#
# Re-run anytime to update; if the installed version matches the latest
# release, the script exits without changes.
#
# Idempotent + safe:
#   * verifies SHA256 of every downloaded asset against the release's SHA256SUMS
#   * picks the binary variant matching the host's installed libcec ABI
#   * never overwrites a service file users have customized via `systemctl edit`
#   * atomic binary swap (.new + chmod + rename)
#
# Override knobs (env):
#   REPO          override the GitHub repo (default LukasParke/capi)
#   VERSION       install a specific tag instead of the latest release
#   INSTALL_DIR   /opt/capi by default
#   FORCE_LIBCEC  override libcec major detection (e.g. FORCE_LIBCEC=7)
#   SKIP_DEPS=1   skip apt-get install of runtime libcec/cec-utils
set -euo pipefail

REPO="${REPO:-LukasParke/capi}"
INSTALL_DIR="${INSTALL_DIR:-/opt/capi}"
SERVICE_PATH="/etc/systemd/system/capi.service"
UDEV_PATH="/etc/udev/rules.d/99-cec.rules"
ENV_FILE="/etc/default/capi"

if [[ "$(id -u)" -ne 0 ]]; then
  echo "ERROR: run as root (sudo)."
  exit 1
fi

# ── Detect host architecture ─────────────────────────────────────────────
ARCH_RAW="$(uname -m)"
case "$ARCH_RAW" in
  aarch64)         ARCH="arm64" ;;
  armv7l|armv6l)   ARCH="armv6" ;;
  *)
    echo "ERROR: unsupported architecture: $ARCH_RAW"
    exit 1
    ;;
esac

# ── Detect libcec major version ──────────────────────────────────────────
detect_libcec_major() {
  if [[ -n "${FORCE_LIBCEC:-}" ]]; then
    echo "$FORCE_LIBCEC"; return
  fi
  # Prefer the runtime soname; fall back to dpkg, then apt-cache.
  for so in /usr/lib/*/libcec.so.* /usr/lib/libcec.so.*; do
    [[ -e "$so" ]] || continue
    case "$so" in
      *.so.6*) echo 6; return ;;
      *.so.7*) echo 7; return ;;
    esac
  done
  if command -v dpkg >/dev/null 2>&1; then
    if dpkg -l libcec6 2>/dev/null | grep -q '^ii'; then echo 6; return; fi
    if dpkg -l libcec7 2>/dev/null | grep -q '^ii'; then echo 7; return; fi
  fi
  if command -v apt-cache >/dev/null 2>&1; then
    if apt-cache --quiet=1 show libcec6 >/dev/null 2>&1; then echo 6; return; fi
    if apt-cache --quiet=1 show libcec7 >/dev/null 2>&1; then echo 7; return; fi
  fi
  echo "ERROR: cannot detect libcec major version. Pass FORCE_LIBCEC=6|7 to override." >&2
  exit 1
}

LIBCEC_MAJOR="$(detect_libcec_major)"
BINARY="capi-linux-${ARCH}-libcec${LIBCEC_MAJOR}"

# Backwards-compat: older releases shipped capi-linux-arm64 (no -libcec suffix).
LEGACY_BINARY="capi-linux-${ARCH}"

echo "==> Architecture:    $ARCH_RAW -> $ARCH"
echo "==> libcec ABI:      $LIBCEC_MAJOR"
echo "==> Target binary:   $BINARY"

# ── Discover release ─────────────────────────────────────────────────────
if [[ -n "${VERSION:-}" ]]; then
  RELEASE_URL="https://api.github.com/repos/${REPO}/releases/tags/${VERSION}"
  echo "==> Pinning to tag:  $VERSION"
else
  RELEASE_URL="https://api.github.com/repos/${REPO}/releases/latest"
fi

echo "==> Fetching release metadata"
RELEASE_JSON="$(curl -fsSL --connect-timeout 10 --max-time 30 "$RELEASE_URL")"

REMOTE_VERSION="$(echo "$RELEASE_JSON" | grep '"tag_name"' | head -1 | cut -d '"' -f 4)"
if [[ -z "$REMOTE_VERSION" ]]; then
  echo "ERROR: could not parse latest release version."
  exit 1
fi
echo "==> Release version: $REMOTE_VERSION"

LOCAL_VERSION=""
if [[ -x "$INSTALL_DIR/capi" ]]; then
  LOCAL_VERSION="$("$INSTALL_DIR/capi" -version 2>/dev/null || true)"
fi
if [[ -n "$LOCAL_VERSION" ]]; then
  echo "==> Installed:       $LOCAL_VERSION"
  if [[ "$LOCAL_VERSION" == "$REMOTE_VERSION" ]]; then
    echo "==> Already up to date. Nothing to do."
    exit 0
  fi
  echo "==> Update available: $LOCAL_VERSION -> $REMOTE_VERSION"
  MODE="update"
else
  echo "==> Performing fresh install."
  MODE="install"
fi

asset_url() {
  echo "$RELEASE_JSON" \
    | grep "browser_download_url.*/$1\"" \
    | head -1 | cut -d '"' -f 4
}

# Resolve the binary URL, falling back to the legacy filename for old releases
# that didn't have the -libcec${N} suffix.
BINARY_URL="$(asset_url "$BINARY")"
if [[ -z "$BINARY_URL" ]]; then
  echo "==> $BINARY not in release; trying legacy $LEGACY_BINARY"
  BINARY_URL="$(asset_url "$LEGACY_BINARY")"
  if [[ -z "$BINARY_URL" ]]; then
    echo "ERROR: release $REMOTE_VERSION has no asset $BINARY (or $LEGACY_BINARY)."
    echo "       Browse https://github.com/${REPO}/releases/tag/${REMOTE_VERSION}"
    exit 1
  fi
  BINARY="$LEGACY_BINARY"
fi

SUMS_URL="$(asset_url "SHA256SUMS")"
SERVICE_URL="$(asset_url "capi.service")"
UDEV_URL="$(asset_url "99-cec.rules")"

if [[ -z "$SERVICE_URL" || -z "$UDEV_URL" ]]; then
  echo "ERROR: release is missing capi.service or 99-cec.rules"
  exit 1
fi

# ── Download into a scratch dir, verify, then install ────────────────────
STAGE="$(mktemp -d -t capi-install.XXXXXX)"
trap 'rm -rf "$STAGE"' EXIT

download() {
  local url="$1" dest="$2" label="$3"
  echo "==> Downloading $label"
  if ! curl -fsSL --retry 3 --retry-delay 2 \
        --connect-timeout 10 --max-time 600 "$url" -o "$dest"; then
    echo "ERROR: download failed: $url"
    exit 1
  fi
}

download "$BINARY_URL"  "$STAGE/$BINARY"        "$BINARY"
download "$SERVICE_URL" "$STAGE/capi.service"   "capi.service"
download "$UDEV_URL"    "$STAGE/99-cec.rules"   "99-cec.rules"

if [[ -n "$SUMS_URL" ]]; then
  download "$SUMS_URL" "$STAGE/SHA256SUMS" "SHA256SUMS"
  echo "==> Verifying checksums"
  ( cd "$STAGE" && sha256sum -c SHA256SUMS --ignore-missing )
else
  echo "WARN: release does not include SHA256SUMS; skipping checksum verification."
  echo "      Expect this only for releases predating the checksum rollout."
fi

# ── Runtime deps (fresh install only, unless SKIP_DEPS=1) ────────────────
if [[ "$MODE" == "install" && "${SKIP_DEPS:-0}" != "1" ]]; then
  echo "==> Installing runtime dependencies"
  apt-get update
  if ! apt-cache --quiet=1 show "libcec${LIBCEC_MAJOR}" >/dev/null 2>&1; then
    echo "ERROR: libcec${LIBCEC_MAJOR} is not available from apt on this system."
    echo "       Install it manually or rerun with FORCE_LIBCEC=<other> to use a different binary variant."
    exit 1
  fi
  apt-get install -y "libcec${LIBCEC_MAJOR}" cec-utils && apt-get clean
fi

# ── Service user + install dir ───────────────────────────────────────────
if ! id -u capi >/dev/null 2>&1; then
  useradd --system --user-group --no-create-home --shell /usr/sbin/nologin capi
  echo "==> Created system user 'capi'"
fi

mkdir -p "$INSTALL_DIR"
chown capi:capi "$INSTALL_DIR"
chmod 0755 "$INSTALL_DIR"

# ── Stop service, atomic binary swap ─────────────────────────────────────
if systemctl is-active --quiet capi.service 2>/dev/null; then
  echo "==> Stopping capi.service for upgrade"
  systemctl stop capi.service
fi

install -o capi -g capi -m 0755 "$STAGE/$BINARY" "$INSTALL_DIR/capi.new"
mv -f "$INSTALL_DIR/capi.new" "$INSTALL_DIR/capi"

# ── Service file: never clobber a customized one ─────────────────────────
service_changed=0
if [[ ! -f "$SERVICE_PATH" ]]; then
  install -m 0644 "$STAGE/capi.service" "$SERVICE_PATH"
  service_changed=1
elif ! cmp -s "$STAGE/capi.service" "$SERVICE_PATH"; then
  if [[ -d "/etc/systemd/system/capi.service.d" ]]; then
    echo "==> Existing capi.service has overrides in /etc/systemd/system/capi.service.d/; leaving the unit file unchanged."
  else
    install -m 0644 "$STAGE/capi.service" "$SERVICE_PATH"
    service_changed=1
  fi
fi

# Provide /etc/default/capi as a place users can put extra flags via
# EnvironmentFile= (referenced by the unit when present).
if [[ ! -f "$ENV_FILE" ]]; then
  cat >"$ENV_FILE" <<EOF
# capi runtime environment (sourced by capi.service via EnvironmentFile)
# Add extra arguments here, e.g.:
# CAPI_EXTRA_FLAGS=-mqtt-broker tcp://localhost:1883 -mqtt-prefix capi
CAPI_EXTRA_FLAGS=
EOF
  chmod 0644 "$ENV_FILE"
fi

# udev rules: always refresh.
install -m 0644 "$STAGE/99-cec.rules" "$UDEV_PATH"
udevadm control --reload-rules

# ── Reload, enable, start ────────────────────────────────────────────────
if [[ "$service_changed" == "1" ]]; then
  systemctl daemon-reload
fi
systemctl enable capi.service >/dev/null 2>&1 || true
systemctl restart capi.service

# ── Health check ─────────────────────────────────────────────────────────
sleep 1
if systemctl is-active --quiet capi.service; then
  echo
  if [[ "$MODE" == "update" ]]; then
    echo "==> capi updated to $REMOTE_VERSION and restarted."
  else
    echo "==> capi installed and running."
  fi
  IP="$(hostname -I 2>/dev/null | awk '{print $1}')"
  if [[ -n "$IP" ]]; then
    echo "==> http://${IP}:8080"
  fi
else
  echo "ERROR: capi.service failed to start. Check: journalctl -u capi.service -n 50"
  exit 1
fi
