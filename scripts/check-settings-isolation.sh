#!/usr/bin/env bash
# Enforce the settings-subsystem boundary: only the allowed modules may read
# from `SettingRepository` directly. All other consumers must go through
# `SettingsReader` so that runtime changes (ApplyMode) take effect without
# restart.
#
# Allowed locations:
#   crates/archivis-core/src/settings/**
#   crates/archivis-api/src/settings/**
#   crates/archivis-db/** (repository itself)
#
# Exits non-zero when a violation is detected.

set -euo pipefail

cd "$(dirname "$0")/.."

# Patterns that indicate a direct DB read/write of the settings table.
PATTERNS=(
  'SettingRepository::get'
  'SettingRepository::get_all'
  'SettingRepository::set'
  'SettingRepository::delete'
  'SettingRepository::set_many'
)

# Files explicitly allowed to use these patterns.
ALLOWED=(
  '^crates/archivis-core/src/settings/'
  '^crates/archivis-api/src/settings/'
  '^crates/archivis-db/'
)

violations=0

for pattern in "${PATTERNS[@]}"; do
  # `grep -r --include '*.rs'` keeps the search fast and focused.
  while IFS= read -r line; do
    file="${line%%:*}"
    allowed=false
    for allow in "${ALLOWED[@]}"; do
      if [[ "$file" =~ $allow ]]; then
        allowed=true
        break
      fi
    done
    if ! $allowed; then
      echo "violation: $line"
      violations=$((violations + 1))
    fi
  done < <(grep -rn --include='*.rs' "$pattern" crates/ 2>/dev/null || true)
done

if [ "$violations" -gt 0 ]; then
  echo >&2
  echo >&2 "Settings isolation check failed: $violations direct SettingRepository call(s)"
  echo >&2 "found outside the allowed modules. Use SettingsReader instead."
  exit 1
fi

echo "settings-isolation: no direct SettingRepository usage outside allowed modules"
