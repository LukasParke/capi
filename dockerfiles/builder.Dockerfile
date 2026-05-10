# Builder image for capi cross-builds.
#
# Parameterized by:
#   BASE_IMAGE  - debian:bookworm (libcec6) or debian:trixie (libcec7)
#   GO_TARBALL  - the linux/<arch> Go release tarball name
#
# The image is run under QEMU when the host architecture differs from the
# target (e.g. amd64 dev machine running an arm64 container). Install
# qemu-user-static + binfmt-support on Debian/Ubuntu, or run
# `docker run --privileged --rm tonistiigi/binfmt --install all` once on
# any Docker host.

ARG BASE_IMAGE=debian:bookworm
FROM ${BASE_IMAGE}

ARG GO_TARBALL=go1.25.2.linux-arm64.tar.gz

# Build deps: matches what the release workflow installs.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        gcc g++ libc6-dev curl pkg-config \
        libcec-dev libp8-platform-dev libudev-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*

RUN curl -sSL "https://go.dev/dl/${GO_TARBALL}" | tar -C /usr/local -xz

ENV PATH=/usr/local/go/bin:${PATH} \
    CGO_ENABLED=1 \
    CGO_LDFLAGS="-Wl,--no-as-needed -lstdc++ -Wl,--as-needed" \
    GOMODCACHE=/go/pkg/mod \
    GOCACHE=/go/cache

WORKDIR /workspace
