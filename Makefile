CARGO := cargo
BINARY := cfproxy
RELEASE_BIN := target/release/$(BINARY)

# Rust toolchain may not be on PATH — check common locations
ifneq ($(shell which cargo 2>/dev/null),)
  CARGO := cargo
else ifneq ($(wildcard $(HOME)/.cargo/bin/cargo),)
  CARGO := $(HOME)/.cargo/bin/cargo
else ifneq ($(wildcard $(HOME)/.rustup/toolchains/stable-*/bin/cargo),)
  CARGO := $(lastword $(wildcard $(HOME)/.rustup/toolchains/stable-*/bin/cargo))
endif

.PHONY: build run test clean install check fmt lint help

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*##' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

build: ## Build release binary
	$(CARGO) build --release

run: ## Run with PORT (e.g., make run PORT=3000)
	@if [ -z "$(PORT)" ]; then echo "Usage: make run PORT=<port>"; exit 1; fi
	$(CARGO) run --release -- $(PORT)

test: ## Run all tests
	$(CARGO) test

check: ## Type-check without building
	$(CARGO) check

fmt: ## Format code
	$(CARGO) fmt

lint: ## Run clippy lints
	$(CARGO) clippy -- -D warnings

clean: ## Remove build artifacts
	$(CARGO) clean

install: build ## Install to ~/.cargo/bin
	cp $(RELEASE_BIN) $(HOME)/.cargo/bin/$(BINARY) 2>/dev/null || $(CARGO) install --path .
