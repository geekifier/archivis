default:
    @just --list

# ── Build ─────────────────────────────────────────────────────────────────────

# Fast compile check (cargo check)
compile:
    cargo check --workspace --all-targets

# Build all crates (debug)
build:
    cargo build --workspace

# Build in release mode
build-release:
    cargo build --workspace --release

# ── Quality ───────────────────────────────────────────────────────────────────

# Format code
fmt:
    cargo fmt --all

# Check formatting (no changes)
fmt-check:
    cargo fmt --all -- --check

# Run clippy lints
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run all backend tests
test:
    cargo test --workspace

# Run frontend unit tests
test-frontend:
    cd frontend && npm test

# Run frontend tests in watch mode
test-frontend-watch:
    cd frontend && npm run test:watch

# Run cargo-deny license and advisory audit
deny:
    cargo deny check

# Full quality gate: fmt + clippy + test + deny
check: fmt-check clippy test deny

# Full frontend gate: build + lint + check + test
check-frontend:
    cd frontend && npm run build && npm run lint && npm run check && npm test

# ── Database ──────────────────────────────────────────────────────────────────

# Run database migrations (requires DATABASE_URL)
migrate:
    cargo sqlx migrate run --source crates/archivis-db/migrations

# Prepare sqlx offline query data for CI
sqlx-prepare:
    cargo sqlx prepare --workspace -- --all-targets

# ── Dev Server (persistent data in .local/data) ──────────────────────────────

# Run backend + frontend dev servers
dev:
    #!/usr/bin/env bash
    trap 'kill 0' EXIT
    cargo run --package archivis-server -- --data-dir .local/data &
    cd frontend && npm run dev &
    wait

# Run backend only with local dev data
dev-backend:
    cargo run --package archivis-server -- --data-dir .local/data

# Run frontend dev server only (expects backend on :9514)
dev-frontend:
    cd frontend && npm run dev

# ── Dev Clean (disposable data in .local/clean) ──────────────────────────────

# Wipe → backend + frontend + create admin (prints creds)
dev-clean:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    export ARCHIVIS_DATA_DIR=".local/clean"
    ./scripts/dev-boot.sh wipe
    cargo run --package archivis-server -- --data-dir .local/clean &
    cd frontend && npm run dev &
    sleep 1
    ./scripts/dev-boot.sh setup
    wait

# Resume existing clean instance data: backend + frontend (no wipe/setup)
dev-clean-resume:
    #!/usr/bin/env bash
    trap 'kill 0' EXIT
    cargo run --package archivis-server -- --data-dir .local/clean &
    cd frontend && npm run dev &
    wait

# Wipe → backend only + create admin (no frontend)
dev-clean-backend:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    export ARCHIVIS_DATA_DIR=".local/clean"
    ./scripts/dev-boot.sh wipe
    cargo run --package archivis-server -- --data-dir .local/clean &
    ./scripts/dev-boot.sh setup
    wait

# Wipe → backend + frontend + admin + seed test data
dev-clean-seed:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    export ARCHIVIS_DATA_DIR=".local/clean"
    ./scripts/dev-boot.sh wipe
    cargo run --package archivis-server -- --data-dir .local/clean &
    cd frontend && npm run dev &
    sleep 1
    ./scripts/dev-boot.sh setup
    ./scripts/dev-boot.sh seed
    wait

# ── Dev Data ──────────────────────────────────────────────────────────────────

# Seed test data into a running instance
dev-seed dir=".local/test-existing" data-dir=".local/clean":
    #!/usr/bin/env bash
    set -euo pipefail
    export ARCHIVIS_DATA_DIR="{{ data-dir }}"
    ./scripts/dev-boot.sh seed "{{ dir }}"

# Reset DB only in .local/data/ (preserves book files)
dev-reset-db:
    #!/usr/bin/env bash
    set -euo pipefail
    export ARCHIVIS_DATA_DIR=".local/data"
    ./scripts/dev-boot.sh wipe-db
    echo "DB wiped. Restart the server to see setup screen."

# Full wipe of .local/data/
dev-reset:
    #!/usr/bin/env bash
    set -euo pipefail
    export ARCHIVIS_DATA_DIR=".local/data"
    ./scripts/dev-boot.sh wipe
    echo "Data directory wiped. Restart the server to start fresh."

# ── E2E Testing ──────────────────────────────────────────────────────────────

# Run Playwright E2E tests (starts backend + frontend automatically)
test-e2e:
    cd frontend && npx playwright test

# Run Playwright E2E tests with interactive UI
test-e2e-ui:
    cd frontend && npx playwright test --ui

# Install Playwright browsers
playwright-install:
    cd frontend && npx playwright install --with-deps chromium

# ── CI ────────────────────────────────────────────────────────────────────────

# Run full CI pipeline locally via act (requires Docker)
ci-local *args:
    act {{ args }}

# Run a single CI job locally via act (e.g. just ci-job fmt)
ci-job job:
    act -j {{ job }}
