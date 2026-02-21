# Archivis

A modern, self-hosted e-book collection manager built with Rust and Svelte 5.

## Prerequisites

- **Rust** (stable toolchain) — installed via [rustup](https://rustup.rs/). The pinned toolchain is picked up automatically from `rust-toolchain.toml`.
- **just** — command runner. Install via `cargo install just` or [other methods](https://github.com/casey/just#installation).
- **cargo-deny** — license and advisory audit. Install via `cargo install cargo-deny`.

## Build

```sh
just build           # debug build
just build-release   # release build
```

## Run

```sh
just run
# or directly:
cargo run --package archivis-server -- --help
```

The server accepts configuration via CLI flags, environment variables (`ARCHIVIS_` prefix), or a TOML config file (default: `config.toml`). CLI flags take highest priority.

| Flag | Env var | Default |
|------|---------|---------|
| `--listen-address` | `ARCHIVIS_LISTEN_ADDRESS` | `127.0.0.1` |
| `--port` | `ARCHIVIS_PORT` | `9514` |
| `--data-dir` | `ARCHIVIS_DATA_DIR` | `data` |
| `--book-storage-path` | `ARCHIVIS_BOOK_STORAGE_PATH` | `<data_dir>/books` |
| `--log-level` | `ARCHIVIS_LOG_LEVEL` | `info` |
| `--config` / `-c` | `ARCHIVIS_CONFIG` | `config.toml` |

Example TOML config file:

```toml
listen_address = "0.0.0.0"
port = 9514
data_dir = "/var/lib/archivis"
book_storage_path = "/mnt/books"
log_level = "info"
```

## Test

```sh
just test            # run all tests
just check-all       # fmt + clippy + test + deny (run before pushing)
```

## Project layout

Cargo workspace with crates under `crates/`:

| Crate | Role |
|-------|------|
| `archivis-server` | Binary entrypoint, config, startup |
| `archivis-api` | Axum HTTP handlers |
| `archivis-core` | Domain models and shared types |
| `archivis-db` | SQLite via sqlx |
| `archivis-formats` | Ebook format detection and parsing |
| `archivis-metadata` | Metadata source plugins (stub) |
| `archivis-tasks` | Background job system |
| `archivis-storage` | File storage abstraction |
| `archivis-auth` | Authentication |

## License

AGPL-3.0-or-later
