default:
    @just --list

# Run all checks (fmt, clippy, test, deny)
check-all: fmt-check clippy test deny

# Check compilation
check:
    cargo check --workspace --all-targets

# Build all crates
build:
    cargo build --workspace

# Build in release mode
build-release:
    cargo build --workspace --release

# Run all tests
test:
    cargo test --workspace

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Format code
fmt:
    cargo fmt --all

# Run clippy lints
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run cargo-deny license and advisory audit
deny:
    cargo deny check

# Run database migrations (requires DATABASE_URL)
migrate:
    cargo sqlx migrate run --source crates/archivis-db/migrations

# Run the server
run:
    cargo run --package archivis-server
