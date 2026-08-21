#!/usr/bin/env bash
# capi installer: download + verify + install the latest GitHub release.
#
# Re-run anytime to update; if the installed version matches the latest
# release, the script exits without changes.
#
# Idempotent + safe:
#   * MANDATORY SHA256 verification: refuses to install from a release that
#     does not ship SHA256SUMS, and refuses any file missing from the sums
#   * picks the binary variant matching the host's installed libcec ABI
#     (libcec6 = Debian 12 / Pi OS Bookworm, libcec7 = Debian 13+ / Trixie);
#     override with FORCE_LIBCEC=6|7
#   * never overwrites a service file users have customized via `systemctl edit`
#   * backs up the previous binary to /opt/capi/capi.bak before swapping
#   * health gate after start (10 x 1s against /api/health); on failure the
#     previous binary is restored and the service restarted (rollback)
#   * runs `udevadm trigger` so already-plugged adapters pick up permissions
#   * atomic binary swap (.new + rename)
#
# Override knobs (env):
#   REPO          override the GitHub repo (default LukasParke/capi)
#   VERSION       install a specific tag instead of the latest release
#   INSTALL_DIR   /opt/capi by default
#   FORCE_LIBCEC  override libcec major detection (e.g. FORCE_LIBCEC=7)
#   SKIP_DEPS=1   skip apt-get install of runtime libcec/cec-utils
#   HEALTH_URL    health endpoint polled after start
#                 (default http://127.0.0.1:8080/api/health)
set -euo pipefail

REPO="${REPO:-LukasParke/capi}"
INSTALL_DIR="${INSTALL_DIR:-/opt/capi}"
SERVICE_PATH="/etc/systemd/system/capi.service"
UDEV_PATH="/etc/udev/rules.d/99-cec.rules"
ENV_FILE="/etc/default/capi"
HEALTH_URL="${HEALTH_URL:-http://127.0.0.1:8080/api/health}"
HEALTH_TRIES="${HEALTH_TRIES:-10}"

fail() { echo "ERROR: $*" >&2; exit 1; }

if [[ "$(id -u)" -ne 0 ]]; then
  fail "run as root (sudo)."
fi

# ── Detect host architecture ─────────────────────────────────────────────
ARCH_RAW="$(uname -m)"
case "$ARCH_RAW" in
  aarch64)         ARCH="arm64" ;;
  armv7l|armv6l)   ARCH="armv6" ;;
  *) fail "unsupported architecture: $ARCH_RAW" ;;
esac

# ── Detect libcec major version ──────────────────────────────────────────
detect_libcec_major() {
  if [[ -n "${FORCE_LIBCEC:-}" ]]; then
    echo "$FORCE_LIBCEC"; return
  fi
  # Prefer the runtime soname; fall back to dpkg, then apt-cache.
  local so
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
  fail "cannot detect libcec major version. Pass FORCE_LIBCEC=6|7 to override."
}
# shellcheck disable=SC2312  # detect_libcec_major calls fail() on its own
LIBCEC_MAJOR="$(detect_libcec_major || fail "libcec ABI detection failed")"

BINARY="capi-linux-${ARCH}-libcec${LIBCEC_MAJOR}"

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
RELEASE_JSON="$(curl -fsSL --connect-timeout 10 --max-time 30 "$RELEASE_URL")" \
  || fail "could not fetch release metadata from $RELEASE_URL"

REMOTE_VERSION="$(printf '%s' "$RELEASE_JSON" \
  | grep '"tag_name"' | head -n1 | cut -d '"' -f4 || true)"
[[ -n "$REMOTE_VERSION" ]] || fail "could not parse latest release version."
echo "==> Release version: $REMOTE_VERSION"

LOCAL_VERSION=""
if [[ -x "$INSTALL_DIR/capi" ]]; then
  LOCAL_VERSION="$("$INSTALL_DIR/capi" -version 2>/dev/null || true)"
fi
MODE="install"
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
fi

asset_url() {
  # Empty result means "asset not present in this release"; callers decide
  # whether that is fatal. Never let grep's nonzero exit kill us here.
  printf '%s' "$RELEASE_JSON" \
    | grep "browser_download_url.*/$1\"" | head -n1 | cut -d '"' -f4 || true
}

BINARY_URL="$(asset_url "$BINARY")"
[[ -n "$BINARY_URL" ]] \
  || fail "release $REMOTE_VERSION has no asset named $BINARY. If the detected libcec ABI is wrong, retry with FORCE_LIBCEC=6|7."

# Checksum verification is MANDATORY: a release without SHA256SUMS is treated
# as broken and refused outright.
SUMS_URL="$(asset_url 'SHA256SUMS')"
[[ -n "$SUMS_URL" ]] \
  || fail "release $REMOTE_VERSION does not include SHA256SUMS; refusing to install unverified artifacts."

SERVICE_URL="$(asset_url 'capi.service')"
UDEV_URL="$(asset_url '99-cec.rules')"
[[ -n "$SERVICE_URL" && -n "$UDEV_URL" ]] \
  || fail "release $REMOTE_VERSION is missing capi.service or 99-cec.rules."

# ── Download into a scratch dir, verify, then install ────────────────────
STAGE="$(mktemp -d -t capi-install.XXXXXX)" || fail "mktemp failed"
trap 'rm -rf "$STAGE"' EXIT

download() {
  local url="$1" dest="$2" label="$3"
  echo "==> Downloading $label"
  curl -fsSL --retry 3 --retry-delay 2 \
       --connect-timeout 10 --max-time 600 "$url" -o "$dest" \
    || fail "download failed: $url"
}

download "$SUMS_URL"     "$STAGE/SHA256SUMS"  "SHA256SUMS"
download "$BINARY_URL"   "$STAGE/$BINARY"     "$BINARY"
download "$SERVICE_URL"  "$STAGE/capi.service" "capi.service"
download "$UDEV_URL"     "$STAGE/99-cec.rules" "99-cec.rules"

echo "==> Verifying checksums"
# Every installed artifact MUST be covered by the sums file; anything absent
# from it is treated as tampered/incomplete.
for f in "$BINARY" capi.service 99-cec.rules; do
  grep -Eq " ${f//./\\.}\$" "$STAGE/SHA256SUMS" \
    || fail "SHA256SUMS has no entry for $f."
done
( cd "$STAGE" && sha256sum -c SHA256SUMS --ignore-missing ) \
  || fail "checksum verification failed; refusing to install."

# ── Runtime deps (fresh install only, unless SKIP_DEPS=1) ────────────────
if [[ "$MODE" == "install" && "${SKIP_DEPS:-0}" != "1" ]]; then
  echo "==> Installing runtime dependencies"
  apt-get update || echo "WARN: apt-get update failed; trying install with stale package lists." >&2
  if ! apt-cache --quiet=1 show "libcec${LIBCEC_MAJOR}" >/dev/null 2>&1; then
    fail "libcec${LIBCEC_MAJOR} is not available from apt on this system. Install it manually or rerun with FORCE_LIBCEC=<other>."
  fi
  apt-get install -y "libcec${LIBCEC_MAJOR}" cec-utils \
    || fail "apt-get install libcec${LIBCEC_MAJOR} cec-utils failed."
  apt-get clean || true
fi

# ── Service user + install dir ───────────────────────────────────────────
if ! id -u capi >/dev/null 2>&1; then
  useradd --system --user-group --no-create-home --shell /usr/sbin/nologin capi \
    || fail "could not create system user 'capi'."
  echo "==> Created system user 'capi'"
fi

mkdir -p "$INSTALL_DIR"
chown capi:capi "$INSTALL_DIR"
chmod 0755 "$INSTALL_DIR"

# ── Stop service, back up, atomic binary swap ────────────────────────────
WAS_ACTIVE=0
if systemctl is-active --quiet capi.service 2>/dev/null; then
  echo "==> Stopping capi.service for upgrade"
  systemctl stop capi.service || fail "could not stop capi.service."
  WAS_ACTIVE=1
fi

HAD_PREVIOUS=0
if [[ -e "$INSTALL_DIR/capi" ]]; then
  cp -a "$INSTALL_DIR/capi" "$INSTALL_DIR/capi.bak" \
    || fail "could not back up $INSTALL_DIR/capi to capi.bak."
  chown capi:capi "$INSTALL_DIR/capi.bak"
  HAD_PREVIOUS=1
  echo "==> Backed up previous binary to $INSTALL_DIR/capi.bak"
fi

install -o capi -g capi -m 0755 "$STAGE/$BINARY" "$INSTALL_DIR/capi.new" \
  || fail "could not stage the new binary."
mv -f "$INSTALL_DIR/capi.new" "$INSTALL_DIR/capi" || fail "binary swap failed."

rollback() {
  echo "==> ROLLING BACK" >&2
  if [[ "$HAD_PREVIOUS" == "1" ]]; then
    mv -f "$INSTALL_DIR/capi.bak" "$INSTALL_DIR/capi" \
      || fail "rollback failed: could not restore $INSTALL_DIR/capi.bak."
    chown capi:capi "$INSTALL_DIR/capi"
    systemctl restart capi.service \
      || echo "ERROR: restored previous binary but restart failed; inspect manually." >&2
    echo "==> Previous binary restored and service restarted."
  else
    systemctl stop capi.service 2>/dev/null || true
    echo "==> Fresh install did not become healthy; service stopped. Remove $INSTALL_DIR and rerun to retry." >&2
  fi
  journalctl -u capi.service -n 50 --no-pager 2>/dev/null || true
}

# ── Service file: never clobber a customized one ─────────────────────────
service_changed=0
if [[ ! -f "$SERVICE_PATH" ]]; then
  install -m 0644 "$STAGE/capi.service" "$SERVICE_PATH" \
    || fail "could not install $SERVICE_PATH."
  service_changed=1
elif ! cmp -s "$STAGE/capi.service" "$SERVICE_PATH"; then
  if [[ -d "/etc/systemd/system/capi.service.d" ]]; then
    echo "==> Existing capi.service has overrides in /etc/systemd/system/capi.service.d/; leaving the unit file unchanged."
  else
    install -m 0644 "$STAGE/capi.service" "$SERVICE_PATH" \
      || fail "could not update $SERVICE_PATH."
    service_changed=1
  fi
fi

# Provide /etc/default/capi as a place users can put extra flags via
# EnvironmentFile= (referenced by the unit when present).
if [[ ! -f "$ENV_FILE" ]]; then
  cat >"$ENV_FILE" <<'EOF'
# capi runtime environment (sourced by capi.service via EnvironmentFile)
# Add extra arguments here, e.g.:
# CAPI_EXTRA_FLAGS=-mqtt-broker tcp://localhost:1883 -mqtt-prefix capi
#
# Optional bearer-token auth: when a token is configured, every /api route
# except /api/health requires it (Authorization: Bearer <token>, X-Auth-Token,
# ?key=<token>, or the capi_token cookie set by the UI login page).
# NOTE: systemd does NOT expand variables across lines, so put the token
# value inline in CAPI_EXTRA_FLAGS via the -token flag:
#   CAPI_TOKEN=change-me          (documentation only; not read directly)
#   CAPI_EXTRA_FLAGS=-token change-me
CAPI_EXTRA_FLAGS=
EOF
  chmod 0644 "$ENV_FILE"
fi

# ── udev rules: refresh + apply to already-plugged adapters ──────────────
install -m 0644 "$STAGE/99-cec.rules" "$UDEV_PATH" \
  || fail "could not install $UDEV_PATH."
udevadm control --reload-rules || echo "WARN: udevadm control --reload-rules failed." >&2
# Re-play uevents so adapters plugged in BEFORE this run get the new
# group/mode applied without needing a replug or reboot.
udevadm trigger || echo "WARN: udevadm trigger failed; replug the adapter once to apply permissions." >&2

# ── Reload, enable, start ────────────────────────────────────────────────
if [[ "$service_changed" == "1" ]]; then
  systemctl daemon-reload || fail "systemctl daemon-reload failed."
fi
systemctl enable capi.service >/dev/null 2>&1 || true
systemctl restart capi.service || fail "systemctl restart capi.service failed."

# ── Health gate: retry loop, rollback on failure ─────────────────────────
health_ok=0
for _ in $(seq 1 "$HEALTH_TRIES"); do
  if curl -fsS -o /dev/null --connect-timeout 2 --max-time 3 "$HEALTH_URL" 2>/dev/null; then
    health_ok=1
    break
  fi
  if [[ "$WAS_ACTIVE" == "1" ]] && ! systemctl is-active --quiet capi.service 2>/dev/null; then
    break   # unit died; no point waiting out the remaining retries
  fi
  sleep 1
done

if [[ "$health_ok" != "1" ]]; then
  echo "ERROR: capi did not become healthy ($HEALTH_URL) after ${HEALTH_TRIES}s."
  rollback
  exit 1
fi

systemctl is-active --quiet capi.service \
  || fail "health endpoint answered but capi.service is not active."

echo
if [[ "$MODE" == "update" ]]; then
  echo "==> capi updated to $REMOTE_VERSION and healthy."
  echo "==> Previous binary kept at $INSTALL_DIR/capi.bak for manual rollback."
else
  echo "==> capi installed, running, and healthy."
fi
IP="$(hostname -I 2>/dev/null | awk '{print $1}' || true)"
if [[ -n "$IP" ]]; then
  echo "==> http://${IP}:8080"
fi
