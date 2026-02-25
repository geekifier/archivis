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
    cargo run --package archivis-server -- --data-dir .local/data &
    cd frontend && npm run dev &
    wait

# Run frontend dev server only (expects backend on :9514)
dev-frontend:
    cd frontend && npm run dev

# Run backend only with local dev data
dev-backend:
    cargo run --package archivis-server -- --data-dir .local/data

# Run frontend checks (build + lint + typecheck + test)
check-frontend:
    cd frontend && npm run build && npm run lint && npm run check && npm test

# Run frontend tests
test-frontend:
    cd frontend && npm test

# Run frontend tests in watch mode
test-frontend-watch:
    cd frontend && npm run test:watch

# Run CI pipeline locally via act (requires Docker)
ci-local *args:
    act {{ args }}

# Run a single CI job locally via act (e.g. just ci-job fmt)
ci-job job:
    act -j {{ job }}

# ── Dev Workflow ────────────────────────────────────────────────────────────

# Fresh boot: clean slate + backend + frontend + create admin + print creds
dev-fresh:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    export ARCHIVIS_DATA_DIR=".local/fresh"
    ./scripts/dev-boot.sh wipe
    cargo run --package archivis-server -- --data-dir .local/fresh &
    cd frontend && npm run dev &
    sleep 1
    ./scripts/dev-boot.sh setup
    wait

# Fresh boot: backend only (no frontend)
dev-fresh-backend:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    export ARCHIVIS_DATA_DIR=".local/fresh"
    ./scripts/dev-boot.sh wipe
    cargo run --package archivis-server -- --data-dir .local/fresh &
    ./scripts/dev-boot.sh setup
    wait

# Reset DB only in .local/data/ (preserves book files)
dev-reset:
    #!/usr/bin/env bash
    set -euo pipefail
    export ARCHIVIS_DATA_DIR=".local/data"
    ./scripts/dev-boot.sh wipe-db
    echo "DB wiped. Restart the server to see setup screen."

# Full wipe of .local/data/
dev-reset-full:
    #!/usr/bin/env bash
    set -euo pipefail
    export ARCHIVIS_DATA_DIR=".local/data"
    ./scripts/dev-boot.sh wipe
    echo "Data directory wiped. Restart the server to start fresh."

# Seed test data from a directory into a running instance (requires ARCHIVIS_DEV_PASSWORD)
dev-seed dir=".local/test-existing":
    #!/usr/bin/env bash
    set -euo pipefail
    export ARCHIVIS_DATA_DIR=".local/fresh"
    ./scripts/dev-boot.sh seed "{{ dir }}"

# Fresh boot + auto-seed test data in one command
dev-fresh-seed:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    export ARCHIVIS_DATA_DIR=".local/fresh"
    export ARCHIVIS_DEV_PASSWORD="${ARCHIVIS_DEV_PASSWORD:-$(openssl rand -base64 18)}"
    ./scripts/dev-boot.sh wipe
    cargo run --package archivis-server -- --data-dir .local/fresh &
    cd frontend && npm run dev &
    sleep 1
    ./scripts/dev-boot.sh setup
    ./scripts/dev-boot.sh seed
    wait
