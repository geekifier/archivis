# Archivis Project

A modern, self-hosted e-book collection manager built with Rust (Axum) and Svelte 5. Planned features include async metadata retrieval from pluggable sources, automated file ingestion, OCR-based ISBN detection, format-aware organization, bulk editing, e-reader sync, and a REST/OPDS/MCP API — all in a single low-footprint binary backed by embedded SQLite.

This software is a work of art, a magnum opus of what AI Agents can achieve.
Treat is as such.

## Core Architectural Principles

Based on the goals and lessons from prior art:

1. **Separate concerns aggressively.** The API server, background workers, and frontend are independent deployable units that communicate through well-defined interfaces.
2. **Database lives separately from book files.** No storing `.db` files alongside ebooks. The database is managed by the application, period.
3. **Single binary for the common case.** The core application (API + embedded DB + built-in worker) ships as one binary. Optional external workers for heavy processing.
4. **Async by default.** Book imports, metadata fetching, OCR, conversions — all background tasks that never block the API or UI.
5. **Plugin architecture from day one.** Metadata sources, storage backends, auth adapters, and ingestion workflows are all pluggable.
6. **API-first design.** The web UI is just another API consumer. Every capability is exposed through the API.

## When using library/project dependencies

Verify compatibility with AGPLv3 license of this project

## Key References

- Design doc: `../_docs/2_.Design01.md`
- Task plan: `../_docs/3_.MVP_Tasks.md`
- Progress: `.docs/PROGRESS.md` — update after completing tasks, be succinct and token efficient while preserving important info for next agent.

## Project Structure

Cargo workspace with 9 crates under `crates/`:
`archivis-core` (domain models), `archivis-db` (SQLite/sqlx), `archivis-formats` (ebook parsing), `archivis-metadata` (stub), `archivis-tasks` (background jobs), `archivis-storage` (file storage), `archivis-auth`, `archivis-api` (Axum handlers), `archivis-server` (binary entrypoint)

## Configuration

Default port is **9514**. Config layering: compiled defaults → TOML file → `ARCHIVIS_*` env vars → CLI flags.

## Development Commands

- `just check-all` — fmt + clippy + test + deny (run before pushing)
- `just ci-local` / `just ci-job <name>` — run CI via act (Docker required)
- `just deny` — cargo-deny locally (act can't run the deny job under QEMU)

## CI

`.github/workflows/ci.yml` — 4 parallel jobs: fmt, clippy, test, deny. Triggers on push to `master` and PRs.

`SQLX_OFFLINE=true` is set in CI. When adding `sqlx::query!` macros, run `cargo sqlx prepare --workspace` and commit the `.sqlx/` directory.

## Development Flow

- use `gh` to interact with github.com
- Use conventional commits
- Sign commits with `-s`
- After implementing each task block and validation, commit
- Create feature branches for new implementation work
- Squash/amend commits within the feature branch when it makes sense based on scope and goals
- When merging into `master`, DO NOT USE MERGE COMMITS, instead do a "rebase merge"
