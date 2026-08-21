#!/usr/bin/env bash
# Push the Rust source tree to a Raspberry Pi, build it there with cargo +
# libcec on-device, install to /opt/capi with backup + health-gated rollback,
# and restart systemd. Slower fallback for hosts without Docker/QEMU; prefer
# scripts/push-pi.sh (cross-build + binary push) for iteration.
#
# Pi one-time setup: rustup (or distro rustc >= 1.85), pkg-config,
# libcec-dev libp8-platform-dev libudev-dev build-essential.
#
# Repo root .env (gitignored), e.g.:
#   SSH_USER=luke
#   SSH_IP=10.10.10.205
#   SSH_PASSWORD=...          # omit if you use SSH keys only
#   SUDO_PASSWORD=...         # optional; defaults to SSH_PASSWORD for sudo -S
#
# The systemd unit is NOT clobbered: if the installed unit differs from the
# repo copy, this script only warns and points you at install.sh.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ ! -f .env ]]; then
  echo "ERROR: Missing .env in $ROOT (copy .env.example)."
  echo "Add SSH_USER, SSH_IP, and SSH_PASSWORD (or use key-based SSH with no password)."
  exit 1
fi

for f in Cargo.toml Cargo.lock build.rs src; do
  [[ -e "$ROOT/$f" ]] || { echo "ERROR: missing $f in the Rust tree; nothing to build."; exit 1; }
done

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
# StrictHostKeyChecking=accept-new is TOFU: fine against your own LAN Pi, but
# pin the host key out-of-band via CAPI_KNOWN_HOSTS for anything sensitive:
#   CAPI_KNOWN_HOSTS=~/.ssh/capi_pi_known_hosts scripts/deploy-pi.sh
if [[ -n "${CAPI_KNOWN_HOSTS:-}" ]]; then
  SSH_OPTS=( -o ConnectTimeout=30 -o StrictHostKeyChecking=yes
             -o UserKnownHostsFile="${CAPI_KNOWN_HOSTS}" )
else
  SSH_OPTS=( -o ConnectTimeout=30 -o StrictHostKeyChecking=accept-new )
fi

# sshpass -e reads the password from the SSHPASS environment variable so it
# never appears in argv (unlike `sshpass -p`, visible in /proc/*/cmdline).
ssh_wrap() {
  if [[ -n "${SSH_PASSWORD:-}" ]]; then
    if ! command -v sshpass >/dev/null 2>&1; then
      echo "ERROR: sshpass not found but SSH_PASSWORD is set."
      echo "Install sshpass, or remove SSH_PASSWORD and use SSH keys."
      exit 1
    fi
    SSHPASS="$SSH_PASSWORD" sshpass -e "$@"
  else
    "$@"
  fi
}

run_ssh() {
  ssh_wrap ssh "${SSH_OPTS[@]}" "$SSH_TARGET" "$@"
}

echo "==> Syncing Rust tree to ${SSH_TARGET}:~/capi-src ..."
# Only the files the on-device cargo build needs; never the .env secrets.
rsync -az --delete \
  -e "ssh ${SSH_OPTS[*]}" \
  --include='Cargo.toml' \
  --include='Cargo.lock' \
  --include='build.rs' \
  --include='/src/***' \
  --include='/templates/***' \
  --include='/static/***' \
  --exclude='*' \
  "$ROOT/" "${SSH_TARGET}:capi-src/"

VERSION="$(git describe --tags --always --dirty 2>/dev/null || echo dev)"

echo "==> Building and installing on Pi (version ${VERSION}) ..."
# Remote script: PASS is one shell-quoted token from the deploy machine.
run_ssh bash -s <<EOF
${PASS_REMOTE}
set -euo pipefail
cd ~/capi-src

sudo() { printf '%s\n' "\$PASS" | command sudo -S "\$@"; }

# Toolchain: prefer rustup's cargo, fall back to a distro rustc new enough
# for the 2024-edition workspace.
if [ -x "\$HOME/.cargo/bin/cargo" ]; then
  export PATH="\$HOME/.cargo/bin:\$PATH"
fi
command -v cargo >/dev/null 2>&1 || {
  echo "ERROR: cargo not found on the Pi. Install rustup (recommended) or rustc >= 1.85." >&2
  exit 1
}

export CGO_ENABLED=1
export CAPI_VERSION='${VERSION}'
cargo build --release --locked

INSTALL_DIR=/opt/capi
BINARY=\$INSTALL_DIR/capi
NEW=\$INSTALL_DIR/capi.new
BAK=\$INSTALL_DIR/capi.bak

sudo mkdir -p "\$INSTALL_DIR"
install -m 0755 target/release/capi "\$NEW"

# Backup + atomic swap, mirroring install.sh.
if [ -e "\$BINARY" ]; then
  sudo cp -a "\$BINARY" "\$BAK"
  HAD_PREVIOUS=1
else
  HAD_PREVIOUS=0
fi

sudo systemctl stop capi.service 2>/dev/null || true
sudo install -o capi -g capi -m 0755 "\$NEW" "\$BINARY"
rm -f "\$NEW"
sudo systemctl restart capi.service

# Health gate: 10 x 1s against /api/health; roll back on failure.
health_ok=0
for _ in \$(seq 1 10); do
  if curl -fsS -o /dev/null --connect-timeout 2 --max-time 3 http://127.0.0.1:8080/api/health 2>/dev/null; then
    health_ok=1
    break
  fi
  sleep 1
done

if [ "\$health_ok" != "1" ]; then
  echo "ERROR: new binary did not become healthy; rolling back." >&2
  journalctl -u capi.service -n 50 --no-pager || true
  if [ "\$HAD_PREVIOUS" = "1" ]; then
    sudo mv -f "\$BAK" "\$BINARY"
    sudo systemctl restart capi.service
    echo "==> Previous binary restored and service restarted."
  else
    sudo systemctl stop capi.service || true
    echo "==> No previous binary; service stopped." >&2
  fi
  exit 1
fi

# Unit hygiene: never clobber a customized unit; warn on drift instead.
if ! cmp -s capi.service /etc/systemd/system/capi.service 2>/dev/null; then
  echo "WARN: installed /etc/systemd/system/capi.service differs from the repo copy."
  echo "      Run install.sh on the Pi to refresh it (it preserves systemctl edit overrides)."
fi

echo "\$BINARY" -version 2>/dev/null || true
curl -fsS -o /dev/null -w "==> /api/health HTTP %{http_code}\n" http://127.0.0.1:8080/api/health
EOF

echo "==> Done. Open http://${SSH_IP}:8080"
