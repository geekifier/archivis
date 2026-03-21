# Architecture

Archivis is built as a Rust + Svelte 5 application shipped as a single binary:

- **Backend**: Rust with [Axum](https://github.com/tokio-rs/axum) web framework and [Tokio](https://tokio.rs/) async runtime
- **Frontend**: [Svelte 5](https://svelte.dev/) with SvelteKit, Tailwind CSS, and shadcn-svelte components
- **Database**: SQLite with WAL mode for concurrent access — no external database needed
- **Background jobs**: Tokio-based async task system

## Design Principles

1. **Separated concerns** — API server, background workers, and frontend have well-defined interfaces
2. **Database lives separately from book files** — no `.db` files stored alongside your ebooks
3. **All heavy work is async** — imports, metadata, OCR, and conversions run as background tasks
4. **Pluggable architecture** — metadata sources, storage backends, and auth adapters are extensible
5. **API-first** — the web UI is just another API consumer; everything it does, you can do via the API

## Workspace Layout

The backend is a Cargo workspace with 9 crates:

| Crate               | Purpose                               |
| ------------------- | ------------------------------------- |
| `archivis-server`   | Binary entrypoint, config, startup    |
| `archivis-api`      | Axum HTTP handlers and REST endpoints |
| `archivis-core`     | Domain models and shared types        |
| `archivis-db`       | SQLite persistence via sqlx           |
| `archivis-formats`  | Ebook format detection and parsing    |
| `archivis-metadata` | Metadata source plugins               |
| `archivis-tasks`    | Background job system                 |
| `archivis-storage`  | File storage abstraction              |
| `archivis-auth`     | Authentication and authorization      |
