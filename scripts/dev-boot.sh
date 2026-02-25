#!/usr/bin/env bash
# dev-boot.sh — Development workflow helper for Archivis
# Subcommands: wipe, wipe-db, wait, setup, seed
# DEV-ONLY: Not for production use.
set -euo pipefail

# ── Config ──────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

DATA_DIR="${ARCHIVIS_DATA_DIR:-.local/fresh}"
PORT="${ARCHIVIS_PORT:-9514}"
DEV_USERNAME="dev"
DEV_PASSWORD="${ARCHIVIS_DEV_PASSWORD:-}"
MAX_WAIT="${ARCHIVIS_MAX_WAIT:-30}"
BASE_URL="http://127.0.0.1:${PORT}"

# ── Path Safety ─────────────────────────────────────────────────────────────

# Resolve DATA_DIR to absolute (works even if path doesn't exist yet)
if [[ "$DATA_DIR" = /* ]]; then
  ABS_DATA_DIR="$DATA_DIR"
else
  ABS_DATA_DIR="${PROJECT_ROOT}/${DATA_DIR}"
fi
# Normalize (remove /../ etc) — use Python as realpath -m isn't portable
ABS_DATA_DIR="$(python3 -c "import os,sys; print(os.path.normpath(sys.argv[1]))" "$ABS_DATA_DIR")"

ABS_LOCAL_DIR="${PROJECT_ROOT}/.local"

case "$ABS_DATA_DIR" in
  "${ABS_LOCAL_DIR}"/*)  ;;
  *)
    echo "ERROR: DATA_DIR '$ABS_DATA_DIR' is outside ${ABS_LOCAL_DIR}/" >&2
    echo "Refusing destructive operation." >&2
    exit 1
    ;;
esac

# ── Helpers ─────────────────────────────────────────────────────────────────

generate_password() {
  openssl rand -base64 18
}

ensure_password() {
  if [[ -z "$DEV_PASSWORD" ]]; then
    DEV_PASSWORD="$(generate_password)"
  fi
}

# ── Subcommands ─────────────────────────────────────────────────────────────

cmd_wipe() {
  echo "Wiping data directory: ${ABS_DATA_DIR}"
  rm -rf "$ABS_DATA_DIR"
  mkdir -p "$ABS_DATA_DIR"
  echo "Done."
}

cmd_wipe_db() {
  echo "Wiping database files in: ${ABS_DATA_DIR}"
  rm -f "$ABS_DATA_DIR"/archivis.db*
  echo "Done."
}

cmd_wait() {
  echo "Waiting for server at ${BASE_URL} (max ${MAX_WAIT}s)..."
  local elapsed=0
  while (( elapsed < MAX_WAIT )); do
    if curl -sf "${BASE_URL}/api/auth/status" > /dev/null 2>&1; then
      echo "Server is ready."
      return 0
    fi
    sleep 0.5
    elapsed=$((elapsed + 1))
  done
  echo "ERROR: Server did not become ready within ${MAX_WAIT}s" >&2
  exit 1
}

cmd_setup() {
  cmd_wait

  ensure_password

  local body
  body=$(jq -n --arg user "$DEV_USERNAME" --arg pass "$DEV_PASSWORD" \
    '{username: $user, password: $pass}')

  local response
  response=$(curl -sf -X POST "${BASE_URL}/api/auth/setup" \
    -H "Content-Type: application/json" \
    -d "$body" 2>&1) || {
    echo "ERROR: Failed to create admin account (setup may already be done)" >&2
    exit 1
  }

  echo ""
  echo "╔══════════════════════════════════════════╗"
  echo "║       Dev credentials created            ║"
  echo "╠══════════════════════════════════════════╣"
  printf "║  Username: %-29s║\n" "$DEV_USERNAME"
  printf "║  Password: %-29s║\n" "$DEV_PASSWORD"
  echo "╠══════════════════════════════════════════╣"
  echo "║  http://localhost:5173                   ║"
  echo "╚══════════════════════════════════════════╝"
  echo ""
}

cmd_seed() {
  local seed_dir="${1:-.local/test-existing}"

  # Resolve seed dir to absolute
  if [[ "$seed_dir" != /* ]]; then
    seed_dir="${PROJECT_ROOT}/${seed_dir}"
  fi

  if [[ ! -d "$seed_dir" ]]; then
    echo "ERROR: Seed directory does not exist: ${seed_dir}" >&2
    exit 1
  fi

  if [[ -z "$DEV_PASSWORD" ]]; then
    echo "ERROR: ARCHIVIS_DEV_PASSWORD must be set for seeding" >&2
    echo "  (it was generated during setup — re-run with dev-fresh-seed or set it)" >&2
    exit 1
  fi

  echo "Logging in as '${DEV_USERNAME}'..."
  local login_body
  login_body=$(jq -n --arg user "$DEV_USERNAME" --arg pass "$DEV_PASSWORD" \
    '{username: $user, password: $pass}')

  local login_response
  login_response=$(curl -sf -X POST "${BASE_URL}/api/auth/login" \
    -H "Content-Type: application/json" \
    -d "$login_body" 2>&1) || {
    echo "ERROR: Login failed — check password" >&2
    exit 1
  }

  local token
  token=$(echo "$login_response" | jq -r '.token')

  if [[ -z "$token" || "$token" == "null" ]]; then
    echo "ERROR: No token in login response" >&2
    exit 1
  fi

  echo "Importing from: ${seed_dir}"
  local scan_body
  scan_body=$(jq -n --arg path "$seed_dir" '{path: $path}')

  local scan_response
  scan_response=$(curl -sf -X POST "${BASE_URL}/api/import/scan/start" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer ${token}" \
    -d "$scan_body" 2>&1) || {
    echo "ERROR: Import scan failed" >&2
    exit 1
  }

  local task_id
  task_id=$(echo "$scan_response" | jq -r '.task_id')
  echo "Import started (task: ${task_id})"
}

# ── Dispatch ────────────────────────────────────────────────────────────────

case "${1:-help}" in
  wipe)     cmd_wipe ;;
  wipe-db)  cmd_wipe_db ;;
  wait)     cmd_wait ;;
  setup)    cmd_setup ;;
  seed)     cmd_seed "${2:-}" ;;
  help|*)
    echo "Usage: $0 <command> [args]"
    echo ""
    echo "Commands:"
    echo "  wipe       Remove and recreate data directory"
    echo "  wipe-db    Remove only database files from data directory"
    echo "  wait       Wait for server readiness"
    echo "  setup      Wait for server + create dev admin account"
    echo "  seed [dir] Import ebooks from dir (default: .local/test-existing)"
    echo ""
    echo "Environment:"
    echo "  ARCHIVIS_DATA_DIR       Data directory (default: .local/fresh)"
    echo "  ARCHIVIS_PORT           Server port (default: 9514)"
    echo "  ARCHIVIS_DEV_PASSWORD   Admin password (default: random)"
    echo "  ARCHIVIS_MAX_WAIT       Readiness timeout in seconds (default: 30)"
    ;;
esac
