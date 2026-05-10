# capi build + deploy targets.
#
# Local dev (any host):
#   make test                Run go test (capi + cec packages)
#   make bench               Run go benchmarks
#   make build               Native build (./capi-server)
#   make release             Optimized native build
#   make dev                 Native build with -race
#   make cross-build         Cross-compile for ARM (PI_ARCH/PI_LIBCEC env)
#   make push-pi             Cross-build + scp + restart on a Pi (.env)
#   make logs-pi             Tail journalctl on the Pi (.env)
#   make logs-pi-follow      Same, follow mode
#   make deploy-pi           (Slow) Sync source + build on the Pi (.env)
#
# On-target (Pi or other Linux box):
#   make setup               apt install build/runtime deps
#   make install             Build + install systemd service (sudo)
#   make deploy              install + enable + start
#   make uninstall           Stop + remove service
#   make status / logs / restart
#
# Variables:
#   VERSION         passed into -ldflags -X main.version=$(VERSION); defaults to git describe.
#   PI_ARCH         arm64 (default) | armv6   (used by cross-build/push-pi)
#   PI_LIBCEC       6 (default) | 7
#
# Outputs of cross-build land in dist/.

.PHONY: all build release dev install uninstall clean test bench run run-local \
        setup deploy deploy-pi push-pi cross-build logs-pi logs-pi-follow \
        status logs restart deps help

BINARY_NAME    = capi
BUILD_OUTPUT   = capi-server
INSTALL_PATH   = /opt/capi
SERVICE_NAME   = capi.service
SERVICE_PATH   = /etc/systemd/system/$(SERVICE_NAME)
UDEV_RULES     = 99-cec.rules
UDEV_PATH      = /etc/udev/rules.d/$(UDEV_RULES)
ENV_DEFAULTS   = /etc/default/capi
VERSION       ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
LDFLAGS_VER    = -ldflags "-X main.version=$(VERSION)"
LDFLAGS_REL    = -ldflags "-X main.version=$(VERSION) -s -w"
CGO_LDFLAGS   ?= -Wl,--no-as-needed -lstdc++ -Wl,--as-needed
PI_ARCH       ?= arm64
PI_LIBCEC     ?= 6

all: build

# ── Native builds ─────────────────────────────────────────────────────────

build:
	@echo "Building $(BINARY_NAME) ($(VERSION))..."
	go mod download
	CGO_LDFLAGS="$(CGO_LDFLAGS)" go build $(LDFLAGS_VER) -o $(BUILD_OUTPUT) ./capi
	@echo "Build complete: ./$(BUILD_OUTPUT)"

release:
	@echo "Building optimized release ($(VERSION))..."
	CGO_LDFLAGS="$(CGO_LDFLAGS)" go build $(LDFLAGS_REL) -o $(BUILD_OUTPUT) ./capi

dev:
	CGO_LDFLAGS="$(CGO_LDFLAGS)" go build -race $(LDFLAGS_VER) -o $(BUILD_OUTPUT) ./capi

# ── Cross-build + push to a Pi ────────────────────────────────────────────

# Cross-compile via Docker + QEMU. Honors PI_ARCH (arm64|armv6) and PI_LIBCEC (6|7).
cross-build:
	@bash scripts/cross-build.sh $(PI_ARCH) $(PI_LIBCEC)

# Fast iteration: cross-build, scp the binary, systemctl restart, curl /api/health.
push-pi:
	@bash scripts/push-pi.sh

# Slow iteration (legacy): rsync the source tree, build on the Pi, restart.
deploy-pi:
	@bash scripts/deploy-pi.sh

logs-pi:
	@bash scripts/pi-logs.sh

logs-pi-follow:
	@FOLLOW=1 bash scripts/pi-logs.sh

# ── On-target install / uninstall ────────────────────────────────────────

setup:
	@echo "Installing system dependencies (Debian/Ubuntu/Raspberry Pi OS)..."
	sudo apt-get update
	sudo apt-get install -y pkg-config libcec-dev libp8-platform-dev libudev-dev cec-utils
	@which go > /dev/null || ( \
		echo "Go not found. Install from https://go.dev/dl/ (linux/arm64 or linux/armv6l for Pi)" && exit 1 \
	)
	@if pkg-config --exists libcec; then \
		echo "libcec dev OK ($$(pkg-config --modversion libcec))"; \
	else \
		echo "ERROR: libcec dev headers missing"; exit 1; \
	fi

install: build
	@echo "Installing $(BINARY_NAME) to $(INSTALL_PATH)..."
	-sudo systemctl stop $(SERVICE_NAME) 2>/dev/null || true
	@id -u capi > /dev/null 2>&1 || \
		(sudo useradd --system --user-group --no-create-home --shell /usr/sbin/nologin capi && echo "Created system user capi.")
	sudo install -d -o capi -g capi -m 0755 $(INSTALL_PATH)
	sudo install -o capi -g capi -m 0755 ./$(BUILD_OUTPUT) $(INSTALL_PATH)/$(BINARY_NAME)
	sudo install -m 0644 capi.service $(SERVICE_PATH)
	sudo install -m 0644 $(UDEV_RULES) $(UDEV_PATH)
	@if [ ! -f $(ENV_DEFAULTS) ]; then \
		printf '# capi runtime environment (sourced via EnvironmentFile)\n# CAPI_EXTRA_FLAGS=-mqtt-broker tcp://localhost:1883\nCAPI_EXTRA_FLAGS=\n' \
			| sudo tee $(ENV_DEFAULTS) > /dev/null; \
		sudo chmod 0644 $(ENV_DEFAULTS); \
	fi
	sudo systemctl daemon-reload
	sudo udevadm control --reload-rules
	@echo
	@echo "Installation complete. To start:"
	@echo "  sudo systemctl enable $(SERVICE_NAME)"
	@echo "  sudo systemctl start  $(SERVICE_NAME)"

deploy: install
	sudo systemctl enable $(SERVICE_NAME)
	sudo systemctl restart $(SERVICE_NAME)
	@echo "Deploy complete. sudo systemctl status $(SERVICE_NAME)"

uninstall:
	-sudo systemctl stop $(SERVICE_NAME)
	-sudo systemctl disable $(SERVICE_NAME)
	sudo rm -f $(SERVICE_PATH) $(UDEV_PATH)
	sudo rm -rf $(INSTALL_PATH)
	sudo systemctl daemon-reload
	sudo udevadm control --reload-rules
	@echo "Uninstall complete (left $(ENV_DEFAULTS) in place)."

# ── Tests + dev server ───────────────────────────────────────────────────

test:
	go test -race ./cec ./capi

bench:
	go test -bench=. -benchmem -run=^$$ -benchtime=1s ./cec ./capi

run: build
	./$(BUILD_OUTPUT)

run-local: build
	./$(BUILD_OUTPUT) -bind localhost:8080

status:
	sudo systemctl status $(SERVICE_NAME)

logs:
	sudo journalctl -u $(SERVICE_NAME) -f

restart:
	sudo systemctl restart $(SERVICE_NAME)

deps:
	@which pkg-config > /dev/null || (echo "pkg-config missing"; exit 1)
	@pkg-config --exists libcec || (echo "libcec dev headers missing"; exit 1)
	@echo "Dependencies OK ($$(pkg-config --modversion libcec))"

clean:
	rm -f $(BUILD_OUTPUT)
	rm -rf dist
	go clean

help:
	@echo "Local dev (any host):"
	@echo "  make test               Run go test -race"
	@echo "  make bench              Run benchmarks"
	@echo "  make build / release / dev"
	@echo "  make cross-build        Cross-compile for ARM (PI_ARCH/PI_LIBCEC env)"
	@echo "  make push-pi            Cross-build, scp, restart on Pi (.env)"
	@echo "  make logs-pi[-follow]   Tail Pi journalctl"
	@echo "  make deploy-pi          (Slow) source-build on Pi"
	@echo
	@echo "On the Pi / target:"
	@echo "  make setup              apt install deps"
	@echo "  make install            Build + install service"
	@echo "  make deploy             install + enable + start"
	@echo "  make uninstall          Remove service"
	@echo "  make status / logs / restart"
	@echo
	@echo "Variables: VERSION=$(VERSION) PI_ARCH=$(PI_ARCH) PI_LIBCEC=$(PI_LIBCEC)"
