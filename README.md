# Archivis

A fast, modern, self-hosted e-book collection manager.

- Import ebooks via upload or directory scan
- Automatic format detection and metadata extraction (EPUB, PDF)
- Background import processing with real-time progress
- Library browsing with grid and list views
- Search (full-text), sort, and filter
- Metadata editing with autocomplete
- REST API with OpenAPI documentation
- Single binary deployment, backed by embedded SQLite
- Dark/light theme

## Quick Start

### Docker (recommended)

```bash
# Create a directory for your books
mkdir -p books

# Start Archivis
docker compose up -d

# Open http://localhost:9514 in your browser
# Create your admin account on first launch
```

Minimal `docker-compose.yml`:

```yaml
services:
  archivis:
    image: archivis/archivis:latest
    build: .
    ports:
      - "9514:9514"
    volumes:
      - archivis-data:/data
      - ./books:/books
    restart: unless-stopped

volumes:
  archivis-data:
```

See `.env.example` for additional configuration options.

### From source

```bash
# Build
cargo build --release --package archivis-server
cd frontend && npm ci && npm run build && cd ..

# Run (serve frontend from build directory)
./target/release/archivis --frontend-dir frontend/build
```

## Configuration

The server accepts configuration via CLI flags, environment variables (`ARCHIVIS_` prefix), or a TOML config file (default: `config.toml`). CLI flags take highest priority.

| Flag                  | Env var                      | Default            |
| --------------------- | ---------------------------- | ------------------ |
| `--listen-address`    | `ARCHIVIS_LISTEN_ADDRESS`    | `127.0.0.1`        |
| `--port`              | `ARCHIVIS_PORT`              | `9514`             |
| `--data-dir`          | `ARCHIVIS_DATA_DIR`          | `data`             |
| `--book-storage-path` | `ARCHIVIS_BOOK_STORAGE_PATH` | `<data_dir>/books` |
| `--frontend-dir`      | `ARCHIVIS_FRONTEND_DIR`      | _(none)_           |
| `--log-level`         | `ARCHIVIS_LOG_LEVEL`         | `info`             |
| `--config` / `-c`     | `ARCHIVIS_CONFIG`            | `config.toml`      |

Example TOML config file:

```toml
listen_address = "0.0.0.0"
port = 9514
data_dir = "/var/lib/archivis"
book_storage_path = "/mnt/books"
log_level = "info"
```

In Docker, `ARCHIVIS_LISTEN_ADDRESS` is set to `0.0.0.0` automatically.

## Development

### Prerequisites

- **Rust** (stable toolchain) — installed via [rustup](https://rustup.rs/). The pinned toolchain is picked up automatically from `rust-toolchain.toml`.
- **Node.js 22+** and npm
- **just** — command runner. Install via `cargo install just` or [other methods](https://github.com/casey/just#installation).
- **cargo-deny** — license and advisory audit. Install via `cargo install cargo-deny`.

### Getting started

```bash
# Install frontend dependencies
cd frontend && npm install && cd ..

# Run development servers (backend + frontend with hot reload)
just dev
```

### Useful commands

| Command               | Description                                        |
| --------------------- | -------------------------------------------------- |
| `just dev`            | Run backend + frontend dev servers                 |
| `just dev-clean`      | Wipe + backend + frontend + create admin           |
| `just dev-clean-seed` | Same as above + seed test data                     |
| `just check`          | fmt + clippy + test + deny (run before pushing)    |
| `just compile`        | Fast compile check (cargo check)                   |
| `just test`           | Run all backend tests                              |
| `just build-release`  | Production build                                   |
| `just sqlx-prepare`   | Regenerate SQLx offline query cache                |
| `just check-frontend` | Lint and typecheck frontend                        |

### Docker development

```bash
docker compose -f docker-compose.yml -f docker-compose.dev.yml up --build
```

## Project layout

Cargo workspace with crates under `crates/`:

| Crate               | Role                               |
| ------------------- | ---------------------------------- |
| `archivis-server`   | Binary entrypoint, config, startup |
| `archivis-api`      | Axum HTTP handlers                 |
| `archivis-core`     | Domain models and shared types     |
| `archivis-db`       | SQLite via sqlx                    |
| `archivis-formats`  | Ebook format detection and parsing |
| `archivis-metadata` | Metadata source plugins (stub)     |
| `archivis-tasks`    | Background job system              |
| `archivis-storage`  | File storage abstraction           |
| `archivis-auth`     | Authentication                     |

## License

Archivis: a modern, self-hosted e-book collection manager.
Copyright (C) 2026 Kamil Markowicz

See [LICENSE](LICENSE) for full license text.

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published
by the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
