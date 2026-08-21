#!/usr/bin/env bash
# Fetch or follow capi.service logs on the Pi via SSH (same .env as deploy-pi.sh).
#
# .env: SSH_USER, SSH_IP, SSH_PASSWORD (optional), SUDO_PASSWORD (optional; defaults to SSH_PASSWORD)
# Env overrides:
#   FOLLOW=1          journalctl -f
#   LINES=200         -n LINES (ignored when FOLLOW=1)
#   CAPI_KNOWN_HOSTS  pin the Pi host key out-of-band instead of TOFU accept-new
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
# accept-new is trust-on-first-use; see .env.example for the CAPI_KNOWN_HOSTS
# alternative that pins the host key ahead of time.
if [[ -n "${CAPI_KNOWN_HOSTS:-}" ]]; then
  SSH_OPTS=( -o ConnectTimeout=30 -o StrictHostKeyChecking=yes
             -o UserKnownHostsFile="${CAPI_KNOWN_HOSTS}" )
else
  SSH_OPTS=( -o ConnectTimeout=30 -o StrictHostKeyChecking=accept-new )
fi

LINES="${LINES:-200}"
FOLLOW="${FOLLOW:-0}"

if ! [[ "$LINES" =~ ^[0-9]+$ ]]; then
  echo "ERROR: LINES must be a non-negative integer"
  exit 1
fi

# sshpass -e reads the password from SSHPASS so it never lands in argv.
ssh_wrap() {
  if [[ -n "${SSH_PASSWORD:-}" ]]; then
    if ! command -v sshpass >/dev/null 2>&1; then
      echo "ERROR: sshpass not found but SSH_PASSWORD is set; install sshpass or use SSH keys."
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
