#!/usr/bin/env bash
# Cross-build capi locally, scp the binary to a Raspberry Pi, restart the
# service, and report health. Designed for the fast dev iteration loop:
# edit code → make push-pi → check Pi.
#
# Reads .env from the repo root. Required:
#   SSH_USER, SSH_IP
# Optional:
#   SSH_PASSWORD       use sshpass-style auth instead of SSH keys
#   SUDO_PASSWORD      defaults to SSH_PASSWORD; needed for sudo -S on the Pi
#   PI_ARCH            arm64 (default) | armv6
#   PI_LIBCEC          6 (default) | 7
#   PI_INSTALL_DIR     /opt/capi (default)
#   PI_SERVICE         capi.service (default)
#
# The script never re-runs apt or rewrites the systemd unit. Use install.sh
# for the first-time install. push-pi.sh is intentionally narrow: replace
# the binary in place and restart.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ -f .env ]]; then
  # shellcheck disable=SC1091
  set -a; source .env; set +a
fi

: "${SSH_USER:?Set SSH_USER in .env}"
: "${SSH_IP:?Set SSH_IP in .env}"

PI_ARCH="${PI_ARCH:-arm64}"
PI_LIBCEC="${PI_LIBCEC:-6}"
PI_INSTALL_DIR="${PI_INSTALL_DIR:-/opt/capi}"
PI_SERVICE="${PI_SERVICE:-capi.service}"

SUDO_PASS="${SUDO_PASSWORD:-${SSH_PASSWORD:-}}"
if [[ -z "$SUDO_PASS" ]]; then
  echo "ERROR: Set SUDO_PASSWORD or SSH_PASSWORD in .env for sudo -S on the Pi."
  exit 1
fi

SSH_TARGET="${SSH_USER}@${SSH_IP}"
SSH_OPTS=( -o ConnectTimeout=15 -o StrictHostKeyChecking=accept-new )

ssh_wrap() {
  if [[ -n "${SSH_PASSWORD:-}" ]]; then
    if ! command -v sshpass >/dev/null 2>&1; then
      echo "ERROR: sshpass not found but SSH_PASSWORD is set; install sshpass or use SSH keys."
      exit 1
    fi
    sshpass -p "$SSH_PASSWORD" "$@"
  else
    "$@"
  fi
}

run_ssh() { ssh_wrap ssh "${SSH_OPTS[@]}" "$SSH_TARGET" "$@"; }
run_scp() { ssh_wrap scp "${SSH_OPTS[@]}" "$@"; }

ARTIFACT="dist/capi-linux-${PI_ARCH}-libcec${PI_LIBCEC}"

echo "==> cross-building $ARTIFACT"
bash scripts/cross-build.sh "$PI_ARCH" "$PI_LIBCEC"

if [[ ! -x "$ARTIFACT" ]]; then
  echo "ERROR: cross-build did not produce $ARTIFACT"
  exit 1
fi

REMOTE_TMP="/tmp/capi.new"
echo "==> scp $ARTIFACT -> ${SSH_TARGET}:${REMOTE_TMP}"
run_scp "$ARTIFACT" "${SSH_TARGET}:${REMOTE_TMP}"

PASS_REMOTE=$(printf 'PASS=%q\n' "$SUDO_PASS")

echo "==> installing + restarting on ${SSH_TARGET}"
run_ssh bash -s <<EOF
${PASS_REMOTE}
set -euo pipefail

sudo() { printf '%s\n' "\$PASS" | command sudo -S "\$@"; }

if [[ ! -d "${PI_INSTALL_DIR}" ]]; then
  sudo mkdir -p "${PI_INSTALL_DIR}"
fi

sudo install -o capi -g capi -m 0755 "${REMOTE_TMP}" "${PI_INSTALL_DIR}/capi"
rm -f "${REMOTE_TMP}"

sudo systemctl restart "${PI_SERVICE}"
sleep 1
systemctl is-active --quiet "${PI_SERVICE}" || (
  echo "ERROR: ${PI_SERVICE} is not active after restart"
  systemctl status "${PI_SERVICE}" --no-pager -n 30
  exit 1
)
"${PI_INSTALL_DIR}/capi" -version || true
curl -fsS -o /dev/null -w "==> /api/health HTTP %{http_code}\n" \
  http://127.0.0.1:8080/api/health || echo "WARN: /api/health not yet responding"
EOF

echo "==> push-pi complete: http://${SSH_IP}:8080"
