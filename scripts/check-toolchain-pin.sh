#!/usr/bin/env bash
# Verify that `rust-toolchain.toml` and every `RUST_TOOLCHAIN` env var in
# `.github/workflows/*.yml` reference the same Rust release. Drift between
# them means rustup ends up with two different toolchain names — `stable`
# and `1.95.0` — and any `rustup target add` from one workflow won't be
# visible to the other when cargo invokes via rust-toolchain.toml.
#
# Also enforces that every workflow that uses `dtolnay/rust-toolchain`
# passes an explicit `toolchain: ${{ env.RUST_TOOLCHAIN }}` input. Without
# it the action silently defaults to "stable", which broke the binary
# build on rust 1.95.0.

set -euo pipefail

cd "$(dirname "$0")/.."

toml_version="$(grep -E '^channel' rust-toolchain.toml | sed -E 's/.*"([^"]+)".*/\1/')"

if [ -z "$toml_version" ]; then
  echo >&2 "could not extract channel from rust-toolchain.toml"
  exit 1
fi

violations=0

# Each workflow that uses dtolnay/rust-toolchain must (1) define
# RUST_TOOLCHAIN matching the toml and (2) pass it via `toolchain:`.
for wf in .github/workflows/*.yml; do
  if ! grep -q 'dtolnay/rust-toolchain' "$wf"; then
    continue
  fi

  ci_version="$(grep -E '^\s*RUST_TOOLCHAIN:' "$wf" | head -1 | sed -E 's/.*"([^"]+)".*/\1/')"
  if [ -z "$ci_version" ]; then
    echo >&2 "${wf}: missing RUST_TOOLCHAIN env var (expected '${toml_version}')"
    violations=$((violations + 1))
    continue
  fi

  if [ "$toml_version" != "$ci_version" ]; then
    echo >&2 "${wf}: RUST_TOOLCHAIN='${ci_version}' but rust-toolchain.toml='${toml_version}'"
    violations=$((violations + 1))
  fi

  # Count dtolnay action usages (only `uses:` lines, not comments) and
  # confirm each has a matching toolchain input.
  dtolnay_count="$(grep -cE 'uses:\s*dtolnay/rust-toolchain' "$wf")"
  passthrough_count="$(grep -c 'toolchain: \${{ env.RUST_TOOLCHAIN }}' "$wf")"
  if [ "$dtolnay_count" -ne "$passthrough_count" ]; then
    echo >&2 "${wf}: ${dtolnay_count} dtolnay/rust-toolchain step(s) but only" \
              "${passthrough_count} pass 'toolchain: \${{ env.RUST_TOOLCHAIN }}'"
    violations=$((violations + 1))
  fi
done

if [ "$violations" -gt 0 ]; then
  echo >&2
  echo >&2 "Toolchain pin check failed: ${violations} drift / config error(s)."
  echo >&2 "Every workflow installing Rust must pin the same version as"
  echo >&2 "rust-toolchain.toml so 'rustup target add' modifies the toolchain"
  echo >&2 "that cargo will actually invoke."
  exit 1
fi

echo "toolchain-pin: rust-toolchain.toml and all workflows pin Rust ${toml_version}"
