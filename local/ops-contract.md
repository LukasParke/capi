# Contract: Ops/deployment/docs overhaul for capi (Rust rewrite)

Repo: /home/luke/github/capi. The Go service is being replaced by a Rust
binary with the SAME name (`capi`), same install dir (/opt/capi/capi), same
config.json, same API. Your job: every deployment, CI, and docs artifact.
Binary artifacts keep the names `capi-linux-arm64-libcec6`,
`capi-linux-arm64-libcec7`, `capi-linux-armv6-libcec6`.

Read first: /home/luke/github/capi/local://review-findings.md (defect list — your work must close every item in its "Deployment/CI" sections), plus current install.sh, capi.service, 99-cec.rules, .github/workflows/release.yml, scripts/*.sh, dockerfiles/builder.Dockerfile, test.sh, README.md, openapi.yaml.

## Files to rewrite

### 1. install.sh (rewrite, keep curl|bash UX)
- `set -euo pipefail` WITHOUT silent-death paths: every `$(...)` capture that can fail gets explicit `|| { echo ...; exit 1; }` handling. No unreachable fallback code.
- SHA256SUMS verification MANDATORY: if the sums file is missing from the release, FAIL LOUDLY (do not warn-and-continue).
- Backup previous binary to /opt/capi/capi.bak before swap; on failed health check, restore backup + restart + exit nonzero (rollback).
- Health gate: retry loop (e.g. 10 x 1s, curl /api/health) instead of single immediate is-active check.
- `udevadm trigger` after installing rules so already-plugged adapters pick up permissions.
- Preserve user drop-in override behavior; if /etc/systemd/system/capi.service.d/ exists, leave unit untouched (existing behavior), else install fresh.
- Create capi user, /etc/default/capi template unchanged. Add `CAPI_TOKEN=` line (commented) documenting the new auth token flag mapping to `-token`.
- Keep arch + libcec ABI detection (libcec6 bookworm / libcec7 trixie, FORCE_LIBCEC override).
- Pin the installer download recommendation in README to a tagged URL (main-branch curl|bash stays as convenience but README shows the pinned variant first).

### 2. capi.service (harden, keep ExecStart shape)
Keep: User/Group=capi, SupplementaryGroups=dialout video, WorkingDirectory, EnvironmentFile=-/etc/default/capi, ExecStart=/opt/capi/capi -bind :8080 -name "CEC HTTP Bridge" $CAPI_EXTRA_FLAGS, Restart=on-failure RestartSec=10, existing sandbox flags. ADD: RestrictAddressFamilies="AF_INET AF_INET6 AF_UNIX", SystemCallFilter=@system-service, CapabilityBoundingSet= (empty), UMask=0077, ProtectClock=true, ProtectHostname=true, ProtectProc=invisible, ProcSubset=pid. Verify ReadWritePaths=/opt/capi stays (config.json + self-update write there). ExecStart flag list must gain nothing mandatory (token optional via env file).

### 3. 99-cec.rules — keep content; add KERNEL qualifiers on the usb-serial rule (nit).

### 4. .github/workflows/release.yml (rewrite for Rust)
- Triggers: push tags v*, workflow_dispatch with bump input, push to main.
- `env:` indirection for EVERY interpolated value used in run: blocks (inputs.bump, needs.tag.outputs.version) — closes the workflow-injection finding.
- Per-job permissions: tag job contents:write; build/release jobs contents:read + release job contents:write for the release upload only.
- Actions pinned to full commit SHAs (look up current SHAs for actions/checkout@v6, docker/setup-qemu-action@v3, actions/upload-artifact@v4, actions/download-artifact@v4, softprops/action-gh-release@v2 — use the latest known SHAs; if unsure use the version tag with a TODO comment, prefer SHA).
- Build matrix: 3 legs (arm64+libcec6 on debian:bookworm, arm64+libcec7 on debian:trixie, armv6+libcec6 on debian:bookworm) via `docker run --platform linux/arm64|linux/arm/v7` QEMU like today. Inside container: apt install cargo via rustup PINNED toolchain (`rustup toolchain install 1.97.1 --profile minimal` — verify checksum by downloading rustup-init with sha256 check against static.rust-lang.org checksums, or use distro rustc if the Debian version is >= 1.85; document choice), apt libcec-dev libp8-platform-dev libudev-dev pkg-config gcc, then `cargo build --release --locked`. Version via env VERSION -> compile-time env!("CAPTI_VERSION") note: the Rust binary reads version from env var CAPI_VERSION at BUILD TIME via option_env!; pass it through.
- concurrency: group "release-${{ github.ref }}" cancel-in-progress: false.
- SHA256SUMS over binaries + install.sh + capi.service + 99-cec.rules (same as now).
- Changelog generation identical semantics; env-indirect the version.

### 5. dockerfiles/builder.Dockerfile — Rust cross builder: ARG BASE_IMAGE default debian:bookworm, digest-pinning comment, rustup with pinned toolchain, cargo-chef NOT required; keep simple.

### 6. scripts/cross-build.sh — same UX (PI_ARCH/PI_LIBCEC, dist/ output) but cargo-based inside the docker builder.

### 7. scripts/push-pi.sh + deploy-pi.sh + pi-logs.sh
- Replace `sshpass -p` with `sshpass -e` + SSHPASS env (closes argv exposure).
- Keep accept-new but add a warning comment + optional CAPI_KNOWN_HOSTS override; document TOFU in .env.example comments.
- deploy-pi.sh: rsync the Rust tree (Cargo.toml, Cargo.lock, src/, build.rs, templates/, static/, proto shim), build on Pi, install with the same backup+rollback as install.sh, stop clobbering the unit unconditionally (use install.sh path or diff-warn).
- .env.example: document SSHPASS/SUDO_PASSWORD usage with sshpass -e.

### 8. test.sh — fix `((passed++))` errexit bug (`x=$((x+1))`), keep structure, add: /api/health, /metrics, auth-rejection check when token set (401 without key, 200 with), SSE headers check (content-type text/event-stream), devices cache shape check. Keep interactive control-tests prompt.

### 9. README.md — full rewrite for the Rust service: same feature set + NEW sections: authentication (token setup, examples with curl -H), update/rollback behavior, MQTT availability topic + LWT, development (cargo build/test/clippy, cross-build), migration note from Go version (config.json compatible, API identical, binary name same). Keep the excellent API reference tables but regenerate them from the route list in local://review-findings.md §"Route table" and openapi.yaml. Install quick-start stays curl|bash + pinned-release variant.

### 10. openapi.yaml — regenerate accurately: every route in §"Route table", envelope schema {status enum[success,error], message, data}, auth as optional bearer (securitySchemes: BearerAuth) applied globally with a note that it is active only when a token is configured, correct /devices wait semantics (default immediate cache; ?wait<=10s), full key name enum (get the list from src/cec/types.rs once written — if not present yet, use the 65-name list from Go capi/cec_exec.go keyNameMap), device schema including discovery/polled_at/first_seen_at/last_seen_at/observed_* fields, LogMessage with levels CEC,APP,ERROR,WARN,INFO,DEBUG, /api/update, /metrics (text/plain), SSE /api/events (text/event-stream), WS /api/events/ws, /api/dev/* full surface, keycode 0 documented as REJECTED (must use key:"select").

## Acceptance
- bash -n on every shell script you touch.
- actionlint-style self-review of release.yml (indentation, expressions).
- Every "Deployment/CI" finding in local://review-findings.md explicitly addressed; append a "Closed findings" checklist at the end of your final report mapping finding -> fix location.
- Do NOT touch src/, Cargo.toml, build.rs, or any .go file.
