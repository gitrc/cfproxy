# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

cfproxy is a Rust CLI that exposes localhost services to the internet via Cloudflare Tunnels with a rich ratatui-powered TUI dashboard. It automatically manages the `cloudflared` binary (download, cache, update) and provides real-time connection status, request metrics, and tunnel details.

## Build & Development Commands

```bash
cargo build                  # Dev build
cargo build --release        # Release build
cargo run -- 3000            # Run against local port 3000
cargo test                   # Run all tests (unit + integration)
cargo test <test_name>       # Run a single test
cargo clippy -- -D warnings  # Lint (treat warnings as errors)
cargo fmt                    # Format code
cargo check                  # Type-check without building
RUST_LOG=debug cargo run -- 3000  # Run with debug logging
```

A `Makefile` wraps these: `make build`, `make test`, `make lint`, `make fmt`, `make run PORT=3000`.

## Dependencies

- **Rust toolchain** (1.70+) via rustup
- **cloudflared** is auto-downloaded at runtime; no manual install needed

## Architecture

Async pipeline: CLI args → config → resolve/download `cloudflared` binary → spawn tunnel subprocess → parse stderr into typed `TunnelEvent`s via mpsc channel → drive TUI.

Key modules in `src/`:

- **main.rs** — Entry point, wires components together
- **cli.rs** — CLI argument definitions (clap derive)
- **config.rs** — Builds runtime `Config` from parsed args
- **cloudflared.rs** — `BinaryManager`: locate, cache, or download the cloudflared binary
- **tunnel.rs** — Spawns cloudflared subprocess, parses stderr into `TunnelEvent`s
- **event.rs** — `TunnelEvent` enum (central data type shared across modules)
- **metrics.rs** — Fetches/parses Prometheus metrics from cloudflared's local metrics server
- **ui/** — TUI rendering and event loop (ratatui + crossterm), split into state, render, overlays, requests, detail, helpers
- **proxy.rs** — Local reverse proxy (hyper) that captures HTTP requests for the TUI
- **qr.rs** — QR code generation for tunnel URL
- **har.rs** — HAR file export
- **diff.rs** — Request diff utilities
- **mock.rs** — Mock response rules (matched against incoming requests)
- **settings.rs** — Persistent settings (`~/.config/cfproxy/settings.json`)
- **cloudflare.rs** — Cloudflare API client (tunnels, DNS, ingress management)
- **setup.rs** — Interactive setup wizard for custom domain configuration
- **purge.rs** — Find and clean stale/orphaned tunnels and DNS records
- **doctor.rs** — Diagnostic checks (settings, binary, network, API, tunnel, DNS)
- **error.rs** — Unified error type and `Result` alias

Integration tests live in `tests/integration.rs`.

## CI/CD

- **CI** (`.github/workflows/ci.yml`): Runs on every push/PR — `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`. Must pass before merging.
- **Release** (`.github/workflows/release.yml`): Triggered by pushing a `v*` tag. Builds binaries for 4 targets (macOS x86_64/ARM64, Linux x86_64/ARM64) and creates a GitHub release with the artifacts.

## Release Process

1. Bump `version` in `Cargo.toml`
2. Commit and push
3. `git tag v<version> && git push origin v<version>`
4. Release workflow builds binaries and publishes the GitHub release automatically

Tags cannot be deleted once published (repo rule), so always use a new version number.
