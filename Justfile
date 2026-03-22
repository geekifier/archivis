host := "127.0.0.1"

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
    cargo test --workspace --tests

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

# Full CI-equivalent gate: backend + frontend
check-all: check check-frontend

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
    export ARCHIVIS_ADMIN_USERNAME="${ARCHIVIS_ADMIN_USERNAME:-dev}"
    creds_file=".local/data/.dev-creds"
    if [[ -z "${ARCHIVIS_ADMIN_PASSWORD:-}" && -f "$creds_file" ]]; then
      source "$creds_file"
      export ARCHIVIS_ADMIN_PASSWORD="${DEV_PASSWORD}"
    else
      export ARCHIVIS_ADMIN_PASSWORD="${ARCHIVIS_ADMIN_PASSWORD:-$(openssl rand -base64 18)}"
      mkdir -p .local/data
      cat > "$creds_file" <<CREDS
    DEV_USERNAME=${ARCHIVIS_ADMIN_USERNAME}
    DEV_PASSWORD=${ARCHIVIS_ADMIN_PASSWORD}
    CREDS
    fi
    printf 'Dev admin: %s / %s\n' "$ARCHIVIS_ADMIN_USERNAME" "$ARCHIVIS_ADMIN_PASSWORD"
    cargo run --package archivis-server -- --data-dir .local/data --listen-address {{host}} &
    cd frontend && npm run dev -- --host {{host}} &
    wait

# Run backend only with local dev data
dev-backend:
    #!/usr/bin/env bash
    export ARCHIVIS_ADMIN_USERNAME="${ARCHIVIS_ADMIN_USERNAME:-dev}"
    creds_file=".local/data/.dev-creds"
    if [[ -z "${ARCHIVIS_ADMIN_PASSWORD:-}" && -f "$creds_file" ]]; then
      source "$creds_file"
      export ARCHIVIS_ADMIN_PASSWORD="${DEV_PASSWORD}"
    else
      export ARCHIVIS_ADMIN_PASSWORD="${ARCHIVIS_ADMIN_PASSWORD:-$(openssl rand -base64 18)}"
      mkdir -p .local/data
      cat > "$creds_file" <<CREDS
    DEV_USERNAME=${ARCHIVIS_ADMIN_USERNAME}
    DEV_PASSWORD=${ARCHIVIS_ADMIN_PASSWORD}
    CREDS
    fi
    printf 'Dev admin: %s / %s\n' "$ARCHIVIS_ADMIN_USERNAME" "$ARCHIVIS_ADMIN_PASSWORD"
    cargo run --package archivis-server -- --data-dir .local/data --listen-address {{host}}

# Run frontend dev server only (expects backend on :9514)
dev-frontend:
    cd frontend && npm run dev -- --host {{host}}

# ── Dev Clean (disposable data in .local/clean) ──────────────────────────────

# Wipe → backend + frontend + create admin (prints creds)
dev-clean:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    export ARCHIVIS_DATA_DIR=".local/clean"
    ./scripts/dev-boot.sh kill
    ./scripts/dev-boot.sh wipe
    cargo run --package archivis-server -- --data-dir .local/clean --listen-address {{host}} &
    cd frontend && npm run dev -- --host {{host}} &
    sleep 1
    ./scripts/dev-boot.sh setup
    wait

# Resume existing clean instance data: backend + frontend (no wipe/setup)
dev-clean-resume:
    #!/usr/bin/env bash
    trap 'kill 0' EXIT
    echo "===== Local Dev Credentials ====="
    cargo run --package archivis-server -- --data-dir .local/clean --listen-address {{host}} &
    cat .local/clean/.dev-creds 2>/dev/null || echo "No existing clean creds found"
    cd frontend && npm run dev -- --host {{host}} --clearScreen false &
    wait

# Alias for resuming the clean dev instance (no wipe/setup).
dev-resume: dev-clean-resume

# Resume backend only (no frontend, no wipe/setup). Used by dev-api.
dev-resume-backend:
    cargo run --package archivis-server -- --data-dir .local/clean --listen-address {{host}}

# Wipe → backend only + create admin (no frontend)
dev-clean-backend:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    export ARCHIVIS_DATA_DIR=".local/clean"
    ./scripts/dev-boot.sh kill
    ./scripts/dev-boot.sh wipe
    cargo run --package archivis-server -- --data-dir .local/clean --listen-address {{host}} &
    ./scripts/dev-boot.sh setup
    wait

# Wipe → backend + frontend + admin + seed test data
dev-clean-seed:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    export ARCHIVIS_DATA_DIR=".local/clean"
    ./scripts/dev-boot.sh kill
    ./scripts/dev-boot.sh wipe
    cargo run --package archivis-server -- --data-dir .local/clean --listen-address {{host}} &
    cd frontend && npm run dev -- --host {{host}} &
    sleep 1
    ./scripts/dev-boot.sh setup
    ./scripts/dev-boot.sh seed
    wait

# ── Dev API ───────────────────────────────────────────────────────────────────

# Ensure API calls use an existing clean instance or safely resume it (no wipe).
[private]
_dev-api-ensure-running:
    #!/usr/bin/env bash
    set -euo pipefail

    base_url="${ARCHIVIS_BASE_URL:-http://127.0.0.1:${ARCHIVIS_PORT:-9514}}"
    export ARCHIVIS_DATA_DIR=".local/clean"

    needs_setup() {
      [[ ! -f .local/clean/.dev-creds ]] || return 1
      local status
      status=$(curl -sf "${base_url}/api/auth/status" 2>/dev/null) || return 1
      echo "$status" | jq -e '.setup_required == true' > /dev/null 2>&1
    }

    if curl -sf "${base_url}/api/auth/status" > /dev/null 2>&1; then
      if needs_setup; then
        ./scripts/dev-boot.sh setup
      fi
      exit 0
    fi

    mkdir -p .local/clean
    echo "Dev API not reachable at ${base_url}; starting 'just dev-resume' in background..." >&2
    nohup just dev-resume-backend > .local/clean/dev-resume.log 2>&1 &
    ARCHIVIS_PORT="${ARCHIVIS_PORT:-9514}" ./scripts/dev-boot.sh wait

    if needs_setup; then
      ./scripts/dev-boot.sh setup
    fi

# Call local API with dev auth (reads token from .local/clean/.dev-creds)
[positional-arguments]
dev-api *args: _dev-api-ensure-running
    #!/usr/bin/env bash
    set -euo pipefail

    creds_file=".local/clean/.dev-creds"
    base_url="${ARCHIVIS_BASE_URL:-http://127.0.0.1:${ARCHIVIS_PORT:-9514}}"

    if [[ ! -f "$creds_file" ]]; then
      echo "ERROR: Missing ${creds_file}" >&2
      echo "Run 'just dev-clean' or 'just dev-clean-backend' first." >&2
      exit 1
    fi

    read_cred() {
      local key="$1"
      awk -F= -v key="$key" '$1 == key { print substr($0, index($0, "=") + 1) }' "$creds_file" | tail -n1
    }

    dev_user="$(read_cred DEV_USERNAME)"
    if [[ -z "$dev_user" ]]; then
      dev_user="dev"
    fi

    token="$(read_cred ARCHIVIS_DEV_TOKEN)"
    if [[ -z "$token" ]]; then
      token="$(read_cred DEV_TOKEN)"
    fi
    if [[ -z "$token" ]]; then
      token="$(read_cred TOKEN)"
    fi

    # Backward compatibility: current .dev-creds stores DEV_PASSWORD only.
    if [[ -z "$token" ]]; then
      dev_password="$(read_cred DEV_PASSWORD)"
      if [[ -z "$dev_password" ]]; then
        echo "ERROR: ${creds_file} must include ARCHIVIS_DEV_TOKEN/DEV_TOKEN/TOKEN or DEV_PASSWORD" >&2
        exit 1
      fi

      login_body="$(jq -nc --arg user "$dev_user" --arg pass "$dev_password" \
        '{username: $user, password: $pass}')"

      login_response="$(
        curl -sf -X POST "${base_url}/api/auth/login" \
          -H "Content-Type: application/json" \
          -d "$login_body"
      )"

      token="$(echo "$login_response" | jq -r '.token')"
      if [[ -z "$token" || "$token" == "null" ]]; then
        echo "ERROR: Failed to obtain dev token from /api/auth/login" >&2
        exit 1
      fi
    fi

    export ARCHIVIS_DEV_TOKEN="$token"

    if [[ "$#" -eq 0 ]]; then
      echo "Usage: just dev-api [curl-args ...] /api/path" >&2
      echo "Example: just dev-api -X GET /api/auth/me" >&2
      exit 1
    fi

    curl_args=("$@")
    has_url=0
    for arg in "${curl_args[@]}"; do
      if [[ "$arg" =~ ^https?:// ]]; then
        has_url=1
        break
      fi
    done

    if (( has_url == 0 )); then
      last_index=$((${#curl_args[@]} - 1))
      if [[ "${curl_args[$last_index]}" == /* ]]; then
        curl_args[$last_index]="${base_url}${curl_args[$last_index]}"
      fi
    fi

    curl -sS --fail-with-body \
      -H "Authorization: Bearer ${ARCHIVIS_DEV_TOKEN}" \
      "${curl_args[@]}"

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

# Run Playwright E2E tests in a visible browser
test-e2e-headed:
    cd frontend && npx playwright test --headed

# Install Playwright browsers
playwright-install:
    cd frontend && npx playwright install --with-deps chromium

# ── Release ──────────────────────────────────────────────────────────────────

# Generate changelog (requires git-cliff: cargo install git-cliff)
changelog:
    git cliff --output CHANGELOG.md

# Preview changelog for next release (unreleased changes)
changelog-preview:
    git cliff --unreleased

# ── Docs ─────────────────────────────────────────────────────────────────────

# Run docs dev server (http://localhost:5175)
docs-dev:
    cd docs && npm run docs:dev

# Build docs for production
docs-build:
    cd docs && npm run docs:build

# Preview production docs build
docs-preview:
    cd docs && npm run docs:preview

# ── CI ────────────────────────────────────────────────────────────────────────

# Run full CI pipeline locally via act (requires Docker)
ci-local *args:
    act {{ args }}

# Run a single CI job locally via act (e.g. just ci-job fmt)
ci-job job:
    act -j {{ job }}
