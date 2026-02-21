# MVP Progress

| Task | Status | Notes |
|------|--------|-------|
| 1.1 Initialize Cargo workspace | Done | 9 crates, workspace deps/lints, `.gitignore` |
| 1.2 Configure dev tooling | Done | `rustfmt.toml`, `clippy.toml`, `rust-toolchain.toml`, `deny.toml`, `Justfile` |
| 1.3 Set up CI pipeline | Done | `.github/workflows/ci.yml` — 4 parallel jobs (fmt, clippy, test, deny). `SQLX_OFFLINE=true`. Local testing via `act` (`.actrc`), `just ci-local` / `just ci-job <name>`. Deny job fails under act/QEMU — use `just deny` locally. |
| 1.4 Application config and startup | Pending | |
