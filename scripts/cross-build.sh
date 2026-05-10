#!/usr/bin/env bash
# Cross-compile capi for ARM via Docker + QEMU.
#
# Usage:
#   scripts/cross-build.sh [ARCH] [LIBCEC]
#     ARCH:   arm64 (default) | armv6
#     LIBCEC: 6     (default) | 7
#
# Produces dist/capi-linux-${ARCH}-libcec${LIBCEC}.
#
# First run for a given (ARCH, LIBCEC) builds the builder image (slow:
# apt install over QEMU). Subsequent runs reuse the image and a named Go
# build cache volume; an incremental build typically completes in <30s on
# a fast x86 machine.
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
    GO_TARBALL="go1.25.2.linux-arm64.tar.gz"
    GOARM=""
    ;;
  armv6)
    PLATFORM="linux/arm/v7"
    GO_TARBALL="go1.25.2.linux-armv6l.tar.gz"
    GOARM="6"
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

# QEMU sanity check: try to ls inside the target-platform image. If it
# fails with "exec format error", binfmt isn't set up.
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
  --build-arg GO_TARBALL="$GO_TARBALL" \
  -f dockerfiles/builder.Dockerfile dockerfiles/

VERSION="$(git describe --tags --always --dirty 2>/dev/null || echo dev)"
mkdir -p dist
OUT="dist/capi-linux-${ARCH}-libcec${LIBCEC}"

echo "==> building $OUT (version $VERSION)"
docker run --rm --platform="$PLATFORM" \
  -v "$ROOT:/workspace" \
  -v capi-go-cache:/go \
  -e GOARM="$GOARM" \
  -e VERSION="$VERSION" \
  "$BUILDER_TAG" \
  bash -c "go build -ldflags \"-X main.version=\${VERSION} -s -w\" -o \"$OUT\" ./capi"

ls -lh "$ROOT/$OUT"
echo "==> built $OUT"
