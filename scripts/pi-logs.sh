#!/usr/bin/env bash
# Fetch or follow capi.service logs on the Pi via SSH (same .env as deploy-pi.sh).
#
# .env: SSH_USER, SSH_IP, SSH_PASSWORD (optional), SUDO_PASSWORD (optional; defaults to SSH_PASSWORD)
# Env overrides:
#   FOLLOW=1    journalctl -f
#   LINES=200   -n LINES (ignored when FOLLOW=1)
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ ! -f .env ]]; then
  echo "ERROR: Missing .env in $ROOT"
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
  echo "ERROR: Set SUDO_PASSWORD or SSH_PASSWORD in .env for journalctl via sudo -S on the Pi."
  exit 1
fi

PASS_REMOTE=$(printf 'PASS=%q\n' "$SUDO_PASS")
SSH_TARGET="${SSH_USER}@${SSH_IP}"
SSH_OPTS=( -o ConnectTimeout=30 -o StrictHostKeyChecking=accept-new )

LINES="${LINES:-200}"
FOLLOW="${FOLLOW:-0}"

if ! [[ "$LINES" =~ ^[0-9]+$ ]]; then
  echo "ERROR: LINES must be a non-negative integer"
  exit 1
fi

sshpass_wrap() {
  if [[ -n "${SSH_PASSWORD:-}" ]]; then
    if command -v sshpass &>/dev/null; then
      sshpass -p "$SSH_PASSWORD" "$@"
    elif [[ -x /tmp/sshpass-local/bin/sshpass ]]; then
      /tmp/sshpass-local/bin/sshpass -p "$SSH_PASSWORD" "$@"
    else
      echo "ERROR: sshpass not found but SSH_PASSWORD is set."
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

echo "==> ${SSH_TARGET}: capi.service logs (FOLLOW=${FOLLOW} LINES=${LINES})"

if [[ "$FOLLOW" == "1" ]]; then
  run_ssh bash -s <<EOF
${PASS_REMOTE}
set -euo pipefail
printf '%s\n' "\$PASS" | sudo -S journalctl -u capi.service --no-pager -o short-iso -f
EOF
else
  run_ssh bash -s <<EOF
${PASS_REMOTE}
set -euo pipefail
printf '%s\n' "\$PASS" | sudo -S journalctl -u capi.service --no-pager -o short-iso -n ${LINES}
EOF
fi
