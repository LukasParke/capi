#!/usr/bin/env bash
# Cross-compile the Rust capi binary for ARM via Docker + QEMU.
#
# Usage:
#   scripts/cross-build.sh [ARCH] [LIBCEC]
#     ARCH:   arm64 (default) | armv6
#     LIBCEC: 6     (default) | 7
#
# Produces dist/capi-linux-${ARCH}-libcec${LIBCEC}.
#
# First run for a given (ARCH, LIBCEC) builds the builder image (slow:
# rustup + apt install over QEMU). Subsequent runs reuse the image and named
# rustup/cargo cache volumes; an incremental build typically completes in
# well under a minute on a fast x86 machine.
#
# Prerequisites: Docker. For non-native architectures, run once:
#   docker run --privileged --rm tonistiigi/binfmt --install all
# (or apt-get install qemu-user-static binfmt-support on Debian/Ubuntu).

set -euo pipefail

ARCH="${1:-arm64}"
LIBCEC="${2:-6}"

case "$ARCH" in
  arm64)
    PLATFORM="linux/arm64"
    # Host toolchain triple inside the emulated container.
    RUST_TRIPLE="aarch64-unknown-linux-gnu"
    RUST_TARGET=""
    ;;
  armv6)
    # The container runs armv7 userland under QEMU, but Pi 1 / Zero are
    # armv6: add the arm-unknown-linux-gnueabihf target and point cargo's
    # linker for it at the container's own gnueabihf gcc so the emitted
    # code is armv6-compatible.
    PLATFORM="linux/arm/v7"
    RUST_TRIPLE="armv7-unknown-linux-gnueabihf"
    RUST_TARGET="arm-unknown-linux-gnueabihf"
    ;;
  *)
    echo "ERROR: unknown ARCH '$ARCH' (expected arm64 or armv6)"
    exit 1
    ;;
esac

case "$LIBCEC" in
  6) BASE_IMAGE="debian:bookworm" ;;
  7) BASE_IMAGE="debian:trixie" ;;
  *)
    echo "ERROR: unknown LIBCEC '$LIBCEC' (expected 6 or 7)"
    exit 1
    ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v docker >/dev/null 2>&1; then
  echo "ERROR: docker not found. Install Docker first."
  exit 1
fi

# QEMU sanity check: try to run a no-op inside the target-platform image. If
# it fails with "exec format error", binfmt isn't set up.
if ! docker run --rm --platform="$PLATFORM" "$BASE_IMAGE" /bin/true 2>/dev/null; then
  echo "ERROR: cannot run $PLATFORM containers on this host."
  echo "Install QEMU binfmt support, e.g.:"
  echo "  docker run --privileged --rm tonistiigi/binfmt --install all"
  echo "  # or on Debian/Ubuntu:"
  echo "  sudo apt-get install qemu-user-static binfmt-support"
  exit 1
fi

BUILDER_TAG="capi-builder:${ARCH}-libcec${LIBCEC}"

echo "==> ensuring builder image $BUILDER_TAG"
docker build --platform="$PLATFORM" \
  -t "$BUILDER_TAG" \
  --build-arg BASE_IMAGE="$BASE_IMAGE" \
  -f dockerfiles/builder.Dockerfile dockerfiles/

VERSION="$(git describe --tags --always --dirty 2>/dev/null || echo dev)"
mkdir -p dist
OUT="dist/capi-linux-${ARCH}-libcec${LIBCEC}"

echo "==> building $OUT (version $VERSION)"
docker run --rm --platform="$PLATFORM" \
  -v "$ROOT:/workspace" \
  -v capi-rustup-cache:/usr/local/rustup \
  -v capi-cargo-cache:/usr/local/cargo \
  -e RUSTUP_HOME=/usr/local/rustup \
  -e CARGO_HOME=/usr/local/cargo \
  -e RUST_TRIPLE="$RUST_TRIPLE" \
  -e RUST_TARGET="$RUST_TARGET" \
  -e CAPI_VERSION="$VERSION" \
  -e VERSION="$VERSION" \
  -e OUT_NAME="$(basename "$OUT")" \
  "$BUILDER_TAG" \
  bash -s <<'BUILD'
set -euxo pipefail
export PATH="/usr/local/cargo/bin:${PATH}"
if [[ -n "${RUST_TARGET}" ]]; then
  rustup target list --installed | grep -qx "${RUST_TARGET}" || rustup target add "${RUST_TARGET}"
  export CARGO_TARGET_ARM_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc
  cargo build --release --locked --target "${RUST_TARGET}"
  cp "target/${RUST_TARGET}/release/capi" "/workspace/dist/${OUT_NAME}"
else
  cargo build --release --locked
  cp target/release/capi "/workspace/dist/${OUT_NAME}"
fi
BUILD

ls -lh "$ROOT/$OUT"
echo "==> built $OUT"
