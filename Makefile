.PHONY: all build install uninstall clean test run setup deploy

# Variables (BUILD_OUTPUT avoids clashing with the capi/ package directory)
BINARY_NAME=capi
BUILD_OUTPUT=capi-server
INSTALL_PATH=/opt/capi
SERVICE_NAME=capi.service
SERVICE_PATH=/etc/systemd/system/$(SERVICE_NAME)
UDEV_RULES_NAME=99-cec.rules
UDEV_RULES_PATH=/etc/udev/rules.d/$(UDEV_RULES_NAME)
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo "dev")
LDFLAGS_VERSION = -ldflags "-X main.version=$(VERSION)"

all: build

# Force link libstdc++ when libcec pulls in static libp8-platform (avoids "DSO missing from command line")
CGO_LDFLAGS ?= -Wl,--no-as-needed -lstdc++ -Wl,--as-needed

# Build the application
build:
	@echo "Building $(BINARY_NAME)..."
	go mod download
	CGO_LDFLAGS="$(CGO_LDFLAGS)" go build $(LDFLAGS_VERSION) -o $(BUILD_OUTPUT) ./capi
	@echo "Build complete: ./$(BUILD_OUTPUT)"

# Build with optimizations
release:
	@echo "Building release version..."
	CGO_LDFLAGS="$(CGO_LDFLAGS)" go build -ldflags "-X main.version=$(VERSION) -s -w" -o $(BUILD_OUTPUT) ./capi
	@echo "Release build complete"

# Install system dependencies (Raspberry Pi OS / Debian / Ubuntu).
# Run this on the Pi (or target device) before make build / make deploy.
setup:
	@echo "Installing system dependencies..."
	sudo apt-get update
	sudo apt-get install -y pkg-config libcec-dev libcec6 cec-utils
	@which go > /dev/null || (echo "Go not found. On Raspberry Pi OS: sudo apt-get install -y golang-go. Or install from https://go.dev/dl/ (choose linux/arm64 or linux/armv6l for Pi)." && exit 1)
	@echo "Dependencies installed."

# Install as system service
install: build
	@echo "Installing $(BINARY_NAME)..."
	-sudo systemctl stop $(SERVICE_NAME) 2>/dev/null
	@id -u capi > /dev/null 2>&1 || \
		(sudo useradd --system --user-group --no-create-home --shell /usr/sbin/nologin capi && echo "Created system user capi.")
	sudo mkdir -p $(INSTALL_PATH)
	sudo cp ./$(BUILD_OUTPUT) $(INSTALL_PATH)/$(BINARY_NAME)
	sudo chmod +x $(INSTALL_PATH)/$(BINARY_NAME)
	sudo cp capi/index.html $(INSTALL_PATH)/index.html
	@echo "Installing systemd service..."
	sudo cp capi.service $(SERVICE_PATH)
	sudo systemctl daemon-reload
	@echo "Installing udev rules for CEC adapter..."
	sudo cp $(UDEV_RULES_NAME) $(UDEV_RULES_PATH)
	sudo udevadm control --reload-rules
	@echo ""
	@echo "Installation complete!"
	@echo "To enable and start the service:"
	@echo "  sudo systemctl enable $(SERVICE_NAME)"
	@echo "  sudo systemctl start $(SERVICE_NAME)"
	@echo ""
	@echo "To check status:"
	@echo "  sudo systemctl status $(SERVICE_NAME)"

# Full deploy: install, enable, and start the service
deploy: install
	@echo "Enabling and starting $(SERVICE_NAME)..."
	sudo systemctl enable $(SERVICE_NAME)
	sudo systemctl start $(SERVICE_NAME)
	@echo "Deploy complete. Service is running."
	@echo "  sudo systemctl status $(SERVICE_NAME)"

# Uninstall service
uninstall:
	@echo "Stopping service..."
	-sudo systemctl stop $(SERVICE_NAME)
	@echo "Disabling service..."
	-sudo systemctl disable $(SERVICE_NAME)
	@echo "Removing files..."
	sudo rm -f $(SERVICE_PATH)
	sudo rm -f $(UDEV_RULES_PATH)
	sudo rm -rf $(INSTALL_PATH)
	sudo systemctl daemon-reload
	sudo udevadm control --reload-rules
	@echo "Uninstall complete"

# Clean build artifacts
clean:
	@echo "Cleaning..."
	rm -f $(BUILD_OUTPUT)
	go clean
	@echo "Clean complete"

# Run tests
test:
	@echo "Running tests..."
	go test ./cec -v

# Run the application locally
run: build
	./$(BUILD_OUTPUT)

# Run with local-only binding
run-local: build
	./$(BUILD_OUTPUT) -bind localhost:8080

# Show service status
status:
	sudo systemctl status $(SERVICE_NAME)

# Show service logs
logs:
	sudo journalctl -u $(SERVICE_NAME) -f

# Restart service
restart:
	sudo systemctl restart $(SERVICE_NAME)

# Development build with race detector
dev:
	CGO_LDFLAGS="$(CGO_LDFLAGS)" go build -race -o $(BUILD_OUTPUT) ./capi

# Check dependencies
deps:
	@echo "Checking dependencies..."
	@which pkg-config > /dev/null || (echo "ERROR: pkg-config not found. Install with: sudo apt-get install pkg-config" && exit 1)
	@pkg-config --exists libcec || (echo "ERROR: libcec not found. Install with: sudo apt-get install libcec-dev" && exit 1)
	@echo "All dependencies satisfied"

# Help (intended for building on the device, e.g. Raspberry Pi)
help:
	@echo "Available targets (run on Raspberry Pi / target device):"
	@echo "  make setup       - Install system dependencies (apt)"
	@echo "  make build       - Build the application"
	@echo "  make release     - Build optimized release version"
	@echo "  make install     - Install as systemd service"
	@echo "  make deploy      - Install, enable, and start service"
	@echo "  make uninstall   - Remove systemd service"
	@echo "  make clean       - Remove build artifacts"
	@echo "  make test        - Run tests"
	@echo "  make run         - Build and run locally"
	@echo "  make run-local   - Build and run on localhost only"
	@echo "  make status      - Show service status"
	@echo "  make logs        - Show service logs (follow mode)"
	@echo "  make restart     - Restart service"
	@echo "  make dev         - Build with race detector"
	@echo "  make deps        - Check dependencies"
	@echo "  make help        - Show this help"
