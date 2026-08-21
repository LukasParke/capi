# Builder image for capi Rust cross-builds.
#
# Parameterized by:
#   BASE_IMAGE    - debian:bookworm (libcec6) or debian:trixie (libcec7)
#
# The image is run under QEMU when the host architecture differs from the
# target (e.g. amd64 dev machine running an arm64 container). Install
# qemu-user-static + binfmt-support on Debian/Ubuntu, or run
# `docker run --privileged --rm tonistiigi/binfmt --install all` once on
# any Docker host.
#
# Digest pinning: for reproducible builds, resolve the current digest and
# build with
#   docker build --build-arg BASE_IMAGE=debian:bookworm@sha256:<digest> ...
# (kept as an ARG rather than hardcoded so bookworm/trixie legs share this
# Dockerfile without divergence).

ARG BASE_IMAGE=debian:bookworm
FROM ${BASE_IMAGE}

ARG RUST_TOOLCHAIN=1.97.1

# Build deps: matches what the release workflow installs.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        gcc libc6-dev curl ca-certificates pkg-config \
        libcec-dev libp8-platform-dev libudev-dev \
 && rm -rf /var/lib/apt/lists/*

# Pinned Rust toolchain via rustup. Debian bookworm's distro rustc is far too
# old (< 1.85); trixie's is newer but we keep one toolchain source everywhere
# so all three release artifacts are built by the same compiler version.
# RUSTUP_HOME/CARGO_HOME live under /usr/local so they can be volume-mounted
# for incremental rebuilds across runs.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:${PATH}

# Verify rustup-init against the published sha256 before running it, then
# install the pinned minimal profile.
RUN set -eux; \
    case "$(uname -m)" in \
      aarch64) triple=aarch64-unknown-linux-gnu ;; \
      armv7l|armv6l|armv8l) triple=armv7-unknown-linux-gnueabihf ;; \
      x86_64) triple=x86_64-unknown-linux-gnu ;; \
      *) echo "unsupported arch: $(uname -m)" >&2; exit 1 ;; \
    esac; \
    curl -fsSL --retry 3 -o /tmp/rustup-init \
      "https://static.rust-lang.org/rustup/dist/${triple}/rustup-init"; \
    curl -fsSL --retry 3 -o /tmp/rustup-init.sha256 \
      "https://static.rust-lang.org/rustup/dist/${triple}/rustup-init.sha256"; \
    echo "$(cat /tmp/rustup-init.sha256)  /tmp/rustup-init" | sha256sum -c -; \
    /tmp/rustup-init -y --no-modify-path --profile minimal \
      --default-toolchain "${RUST_TOOLCHAIN}"; \
    rm -f /tmp/rustup-init /tmp/rustup-init.sha256

ENV CGO_ENABLED=1

WORKDIR /workspace
