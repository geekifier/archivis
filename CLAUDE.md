# Archivis Project

Self-hosted ebook collection manager. Rust (Axum) backend + Svelte 5 frontend, single binary, embedded SQLite.

THIS IS PRERELEASE SOFTWARE. Sound architecture, clean code, and good UX are the top priorities. Do not worry about backwards compatibility when evaluating solutions. Call out gaps you happen to notice incidentally during your work, even if not related to your specific task.

## Architectural Constraints

1. API server, background workers, and frontend are separate concerns with well-defined interfaces
2. Database lives separately from book files
3. Single binary ships API + embedded DB + built-in worker
4. All heavy work (imports, metadata, OCR, conversions) runs as async background tasks
5. Metadata sources, storage backends, auth adapters, ingestion workflows are pluggable
6. API-first: the web UI is just another API consumer

## When using library/project dependencies

Verify compatibility with AGPLv3 license of this project

## Key References

- Design doc: `../_docs/2_.Design01.md`

For debugging, you can query the API: use `just dev-api` followed by curl args. Examples:

- `just dev-api -X GET /api/auth/me`
- `just dev-api -X POST -H 'Content-Type: application/json' -d '{"path":"/tmp/books"}' /api/import/scan/start`

## Project Structure

Cargo workspace with 9 crates under `crates/`:
`archivis-core` (domain models), `archivis-db` (SQLite/sqlx), `archivis-formats` (ebook parsing), `archivis-metadata` (stub), `archivis-tasks` (background jobs), `archivis-storage` (file storage), `archivis-auth`, `archivis-api` (Axum handlers), `archivis-server` (binary entrypoint)
`.local/` (gitignored): `data/` (persistent dev data), `clean/` (disposable, wiped by `dev-clean*` targets), `test-existing` (existing ebook collection), `test-ingestion` (watched folder input), `test-library` (final library storage).

## Configuration

Default port: **9514**. Env vars can override any config setting.

## Development Commands

- `just check` — fmt + clippy + test + deny (run before pushing)
- `just check-frontend` — build + lint + check + test (frontend quality gate)
- `just compile` — fast cargo check (compile only, no lints/tests)
- `just test-e2e` — Playwright E2E tests (auto-starts backend + frontend)
- `just dev-clean-backend` — wipe + backend + admin setup; quickest way to get a testable API
- `just sqlx-prepare` — prepare offline query data (run after query changes, commit `.sqlx/`)
- `just ci-local` / `just ci-job <name>` — run CI via act (Docker required)

## Development Flow

- Use `gh` for GitHub interaction
- Conventional commits, sign with `-s`
- Feature branches for new work; squash/amend within branch when appropriate
- Merge to `master` via rebase merge only (no merge commits)
- NEVER commit gitignored files without explicit user instruction

## Important Rules

- Do not adjust linter rules, security audit configuration or other "guardrails" without explicit instructions or providing the user with an explanation and having the user acknowledge the changes.
- Use backticks around identifiers in code comments
