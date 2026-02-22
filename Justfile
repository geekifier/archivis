default:
    @just --list

# Run all checks (fmt, clippy, test, deny)
check-all: fmt-check clippy test deny

# Check compilation
check:
    cargo check --workspace --all-targets

# Build all crates
build:
    cargo build --workspace

# Build in release mode
build-release:
    cargo build --workspace --release

# Run all tests
test:
    cargo test --workspace

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Format code
fmt:
    cargo fmt --all

# Run clippy lints
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run cargo-deny license and advisory audit
deny:
    cargo deny check

# Run database migrations (requires DATABASE_URL)
migrate:
    cargo sqlx migrate run --source crates/archivis-db/migrations

# Prepare sqlx offline query data for CI
sqlx-prepare:
    cargo sqlx prepare --workspace -- --all-targets

# Run the server
run:
    cargo run --package archivis-server

# Run backend + frontend dev server together
dev:
    #!/usr/bin/env bash
    trap 'kill 0' EXIT
    cargo run --package archivis-server &
    cd frontend && npm run dev &
    wait

# Run frontend dev server only (expects backend on :9514)
dev-frontend:
    cd frontend && npm run dev

# Run frontend checks (build + lint + typecheck)
check-frontend:
    cd frontend && npm run build && npm run lint && npm run check

# Run CI pipeline locally via act (requires Docker)
ci-local *args:
    act {{ args }}

# Run a single CI job locally via act (e.g. just ci-job fmt)
ci-job job:
    act -j {{ job }}
