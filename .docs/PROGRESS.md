# Archivis MVP Progress

## Completed Blocks

### Block 1 — Project Scaffolding
- Cargo workspace with 9 crates, dev tooling, CI pipeline, config/startup

### Block 2 — Domain Models
- Core types: Book, Author, Series, Publisher, BookFile, Identifier, Tag
- Error hierarchy with thiserror derives and From conversions

### Block 3 — Database Layer
- SQLite WAL connection pool, initial schema migration (FTS5, triggers)
- Repository layer: Book, BookFile, Author, Identifier, Series, Tag, Publisher

### Block 4 — Storage Layer
- StorageBackend trait (RPITIT), LocalStorage with atomic writes, SHA-256 hashing
- Path generation with cross-platform sanitization

### Block 5 — Format Detection & Metadata Extraction
- Magic-byte format detection (EPUB, PDF, MOBI, CBZ, FB2, DJVU, TXT)
- EPUB metadata extraction (OPF parsing, cover extraction)
- PDF metadata extraction (info dict + XMP)
- Filename/path parser with regex patterns
- Metadata quality scoring with ISBN validation and garbage detection

### Block 6 — Import Pipeline
- **ImportService** (`archivis-tasks::import`): 9-step single-file import pipeline
  - Format detection, metadata extraction, filename parsing, quality scoring
  - SHA-256 duplicate detection, ISBN duplicate handling (same format = skip, different format = link)
  - File storage via StorageBackend, cover extraction + WebP thumbnail generation (sm/md)
  - DB record creation: Book, BookFile, Authors, Identifiers, Tags, Series
- **BulkImportService**: directory scanning + bulk import
  - Async recursive directory walking, extension + magic-byte filtering
  - ImportManifest with format counts and file sizes
  - Sequential processing with categorized results (imported/skipped/failed)
  - ImportProgress trait for callback-based progress reporting
- 17 tests (5 unit + 12 integration)

### Block 7 — Background Task System
- **Task domain models** in `archivis-core::models::task`: Task, TaskType, TaskStatus, TaskProgress
- **Migration 002_tasks.sql**: `tasks` table with status/progress tracking
- **TaskRepository** (`archivis-db`): CRUD, active/recent listing, progress updates, startup recovery
- **TaskQueue** (`archivis-tasks::queue`): in-process Tokio queue with mpsc dispatch + broadcast progress
  - Worker trait (dyn-safe via Pin<Box<Future>>), ProgressSender (DB + broadcast)
  - `run_dispatcher` loop, `recover_tasks` on startup
- **Import workers** (`archivis-tasks::workers`):
  - ImportFileWorker — wraps ImportService with progress reporting
  - ImportDirectoryWorker — wraps BulkImportService with BroadcastProgress adapter
- **SSE progress endpoints** (`archivis-api::tasks`):
  - `GET /api/tasks` — list recent tasks (JSON)
  - `GET /api/tasks/{id}` — task detail (JSON)
  - `GET /api/tasks/{id}/progress` — SSE stream per task
  - `GET /api/tasks/active` — SSE stream for all active tasks
- **AppState** (`archivis-api::state`): shared state for API handlers (DbPool + TaskQueue)

### Block 8 — Auth System
- **Migration 003_auth.sql**: `users` table (UUID, username, email, password_hash, role, is_active) + `sessions` table (token_hash, expires_at, user_id FK)
- **Domain models** (`archivis-core::models::user`): User, Session, UserRole enum
  - Password hash excluded from JSON serialization (`#[serde(skip_serializing)]`)
- **Repositories** (`archivis-db`): UserRepository (CRUD + count), SessionRepository (token lookup, expiry cleanup)
- **AuthAdapter trait** (`archivis-auth`): RPITIT-based pluggable auth backend
  - `LocalAuthAdapter`: Argon2id password hashing (spawn_blocking), password validation (8+ chars)
  - First user registration auto-assigns Admin role
- **AuthService** (`archivis-auth`): generic over `AuthAdapter`
  - register, login (256-bit token, SHA-256 hashed for DB), validate_session, logout, is_setup_required
  - Session tokens: OsRng 32-byte random → hex → SHA-256 hash stored (raw token to client)
  - 30-day session expiry with automatic cleanup on validation
- **Auth middleware** (`archivis-api::auth`):
  - `AuthUser` extractor: `FromRequestParts` — checks Bearer header then session cookie
  - `RequireAdmin` extractor: chains AuthUser + role check
  - `AuthRejection`: JSON error responses (401/403)
- **AppState** updated: now holds `AuthService<LocalAuthAdapter>`
- 267 total tests passing across workspace (23 new auth tests)
