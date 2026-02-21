# MVP Progress

| Task | Status | Notes |
|------|--------|-------|
| 1.1 Initialize Cargo workspace | Done | 9 crates, workspace deps/lints, `.gitignore` |
| 1.2 Configure dev tooling | Done | `rustfmt.toml`, `clippy.toml`, `rust-toolchain.toml`, `deny.toml`, `Justfile` |
| 1.3 Set up CI pipeline | Done | `.github/workflows/ci.yml` — 4 parallel jobs (fmt, clippy, test, deny). `SQLX_OFFLINE=true`. Local testing via `act` (`.actrc`), `just ci-local` / `just ci-job <name>`. Deny job fails under act/QEMU — use `just deny` locally. |
| 1.4 Application config and startup | Done | `clap` CLI + `figment` layered config (defaults → TOML → env → CLI). `tracing` structured logging. Tokio runtime with signal handling. 8 unit tests. |
| 2.1 Core domain types | Done | `archivis-core::models` — Book, Author, Series, Publisher, BookFile, Identifier, Tag structs. Enums: BookFormat (9 formats w/ extension+MIME), MetadataStatus, IdentifierType, MetadataSource (tagged). Display, Serialize/Deserialize, FromStr for enums. Auto sort_title (strips articles) and sort_name (Last, First). Confidence clamping. 42 unit tests. |
| 2.2 Error types | Done | `archivis-core::errors` — ArchivisError top-level enum, DbError, FormatError, StorageError, AuthError, TaskError, MetadataError. thiserror derives, From conversions, io::Error conversions. `archivis-api::errors::ApiError` with IntoResponse mapping to HTTP status codes (400/401/403/404/409/422/500). Internal details logged but not exposed to clients. 10 API error tests. |
