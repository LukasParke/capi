#!/usr/bin/env bash
# Push this repo to a Raspberry Pi, build with Go + libcec on the Pi, install to /opt/capi, restart systemd.
#
# Pi one-time setup: Go in /usr/local/go, pkg-config, libcec-dev, libcec7, libp8-platform-dev, libudev-dev, build-essential
# (same as a manual on-device build).
#
# Repo root .env (gitignored), e.g.:
#   SSH_USER=luke
#   SSH_IP=10.10.10.205
#   SSH_PASSWORD=...          # omit if you use SSH keys only
#   SUDO_PASSWORD=...           # optional; defaults to SSH_PASSWORD for sudo -S

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ ! -f .env ]]; then
  echo "ERROR: Missing .env in $ROOT"
  echo "Add SSH_USER, SSH_IP, and SSH_PASSWORD (or use key-based SSH with no password)."
  exit 1
fi

# shellcheck disable=SC1091
set -a
source .env
set +a

: "${SSH_USER:?Set SSH_USER in .env}"
: "${SSH_IP:?Set SSH_IP in .env}"

SUDO_PASS="${SUDO_PASSWORD:-${SSH_PASSWORD:-}}"
if [[ -z "$SUDO_PASS" ]]; then
  echo "ERROR: Set SUDO_PASSWORD or SSH_PASSWORD in .env for non-interactive sudo on the Pi."
  exit 1
fi

PASS_REMOTE=$(printf 'PASS=%q\n' "$SUDO_PASS")

SSH_TARGET="${SSH_USER}@${SSH_IP}"
SSH_OPTS=( -o ConnectTimeout=30 -o StrictHostKeyChecking=accept-new )

sshpass_wrap() {
  if [[ -n "${SSH_PASSWORD:-}" ]]; then
    if command -v sshpass &>/dev/null; then
      sshpass -p "$SSH_PASSWORD" "$@"
    elif [[ -x /tmp/sshpass-local/bin/sshpass ]]; then
      /tmp/sshpass-local/bin/sshpass -p "$SSH_PASSWORD" "$@"
    else
      echo "ERROR: sshpass not found but SSH_PASSWORD is set."
      echo "Install sshpass, or remove SSH_PASSWORD and use SSH keys."
      exit 1
    fi
  else
    "$@"
  fi
}

run_ssh() {
  if [[ -n "${SSH_PASSWORD:-}" ]]; then
    sshpass_wrap ssh "${SSH_OPTS[@]}" "$SSH_TARGET" "$@"
  else
    ssh "${SSH_OPTS[@]}" "$SSH_TARGET" "$@"
  fi
}

echo "==> Syncing repo to ${SSH_TARGET}:~/capi-src ..."
tar czf - \
  --exclude='./.git' \
  --exclude='./capi-server' \
  --exclude='./.env' \
  . 2>/dev/null \
  | if [[ -n "${SSH_PASSWORD:-}" ]]; then
      sshpass_wrap ssh "${SSH_OPTS[@]}" "$SSH_TARGET" 'rm -rf ~/capi-src && mkdir -p ~/capi-src && tar xzf - -C ~/capi-src'
    else
      ssh "${SSH_OPTS[@]}" "$SSH_TARGET" 'rm -rf ~/capi-src && mkdir -p ~/capi-src && tar xzf - -C ~/capi-src'
    fi

VERSION="$(git describe --tags --always --dirty 2>/dev/null || echo dev)"

echo "==> Building and installing on Pi (version ${VERSION}) ..."
# Remote script: PASS is one shell-quoted token from the deploy machine.
run_ssh bash -s <<EOF
${PASS_REMOTE}
set -euo pipefail
cd ~/capi-src
export PATH="/usr/local/go/bin:\$PATH"
export CGO_ENABLED=1
export CGO_LDFLAGS='-Wl,--no-as-needed -lstdc++ -Wl,--as-needed'
go build -ldflags "-X main.version=${VERSION} -s -w" -o capi-server ./capi
echo "\$PASS" | sudo -S systemctl stop capi.service 2>/dev/null || true
echo "\$PASS" | sudo -S cp capi-server /opt/capi/capi
echo "\$PASS" | sudo -S chmod +x /opt/capi/capi
echo "\$PASS" | sudo -S cp capi.service /etc/systemd/system/capi.service
echo "\$PASS" | sudo -S systemctl daemon-reload
echo "\$PASS" | sudo -S systemctl restart capi.service
sleep 2
systemctl is-active capi.service
curl -sS -o /dev/null -w "health HTTP %{http_code}\n" http://127.0.0.1:8080/api/health || true
EOF

echo "==> Done. Open http://${SSH_IP}:8080"
