.PHONY: all build install uninstall clean test run

# Variables
BINARY_NAME=capi
INSTALL_PATH=/opt/capi
SERVICE_NAME=capi.service
SERVICE_PATH=/etc/systemd/system/$(SERVICE_NAME)

all: build

# Build the application
build:
	@echo "Building $(BINARY_NAME)..."
	go mod download
	go build -o $(BINARY_NAME) ./capi
	@echo "Build complete: ./$(BINARY_NAME)"

# Build with optimizations
release:
	@echo "Building release version..."
	go build -ldflags="-s -w" -o $(BINARY_NAME) ./capi
	@echo "Release build complete"

# Install as system service
install: build
	@echo "Installing $(BINARY_NAME)..."
	sudo mkdir -p $(INSTALL_PATH)
	sudo cp $(BINARY_NAME) $(INSTALL_PATH)/
	sudo chmod +x $(INSTALL_PATH)/$(BINARY_NAME)
	@echo "Creating systemd service..."
	sudo cp capi.service $(SERVICE_PATH)
	sudo systemctl daemon-reload
	@echo ""
	@echo "Installation complete!"
	@echo "To enable and start the service:"
	@echo "  sudo systemctl enable $(SERVICE_NAME)"
	@echo "  sudo systemctl start $(SERVICE_NAME)"
	@echo ""
	@echo "To check status:"
	@echo "  sudo systemctl status $(SERVICE_NAME)"

# Uninstall service
uninstall:
	@echo "Stopping service..."
	-sudo systemctl stop $(SERVICE_NAME)
	@echo "Disabling service..."
	-sudo systemctl disable $(SERVICE_NAME)
	@echo "Removing files..."
	sudo rm -f $(SERVICE_PATH)
	sudo rm -rf $(INSTALL_PATH)
	sudo systemctl daemon-reload
	@echo "Uninstall complete"

# Clean build artifacts
clean:
	@echo "Cleaning..."
	rm -f $(BINARY_NAME)
	go clean
	@echo "Clean complete"

# Run tests
test:
	@echo "Running tests..."
	go test ./cec -v

# Run the application locally
run: build
	./$(BINARY_NAME)

# Run with local-only binding
run-local: build
	./$(BINARY_NAME) -bind localhost:8080

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
	go build -race -o $(BINARY_NAME) ./capi

# Check dependencies
deps:
	@echo "Checking dependencies..."
	@which pkg-config > /dev/null || (echo "ERROR: pkg-config not found. Install with: sudo apt-get install pkg-config" && exit 1)
	@pkg-config --exists libcec || (echo "ERROR: libcec not found. Install with: sudo apt-get install libcec-dev" && exit 1)
	@echo "All dependencies satisfied"

# Help
help:
	@echo "Available targets:"
	@echo "  make build       - Build the application"
	@echo "  make release     - Build optimized release version"
	@echo "  make install     - Install as systemd service"
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
