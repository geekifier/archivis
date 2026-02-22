# ==============================================================================
# Archivis — Multi-stage Docker build
# Produces a minimal Alpine image with a statically-linked Rust binary and
# pre-built SvelteKit frontend assets.
# ==============================================================================

# ------------------------------------------------------------------------------
# Stage 1: Build the SvelteKit frontend
# ------------------------------------------------------------------------------
FROM node:22-alpine AS frontend

WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ .
RUN npm run build

# ------------------------------------------------------------------------------
# Stage 2: Build the Rust backend (musl -> static binary)
# ------------------------------------------------------------------------------
FROM rust:1-alpine AS backend

# musl-dev is required for static linking on Alpine
RUN apk add --no-cache musl-dev

WORKDIR /app

# Compile-time checked SQL queries without a live database
ENV SQLX_OFFLINE=true

# -- Dependency caching layer --------------------------------------------------
# Copy only manifests and lock file first, then create stub source files so
# `cargo build` resolves and compiles every *dependency* without touching real
# application code. When only source changes, Docker reuses this cached layer.

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./

# Create stub lib.rs / main.rs for each workspace crate
RUN mkdir -p crates/archivis-core/src     && echo "" > crates/archivis-core/src/lib.rs      \
 && mkdir -p crates/archivis-db/src       && echo "" > crates/archivis-db/src/lib.rs        \
 && mkdir -p crates/archivis-formats/src  && echo "" > crates/archivis-formats/src/lib.rs   \
 && mkdir -p crates/archivis-metadata/src && echo "" > crates/archivis-metadata/src/lib.rs  \
 && mkdir -p crates/archivis-tasks/src    && echo "" > crates/archivis-tasks/src/lib.rs     \
 && mkdir -p crates/archivis-storage/src  && echo "" > crates/archivis-storage/src/lib.rs   \
 && mkdir -p crates/archivis-auth/src     && echo "" > crates/archivis-auth/src/lib.rs      \
 && mkdir -p crates/archivis-api/src      && echo "" > crates/archivis-api/src/lib.rs       \
 && mkdir -p crates/archivis-server/src   && echo "fn main() {}" > crates/archivis-server/src/main.rs

# Copy per-crate Cargo.toml files (needed for workspace resolution)
COPY crates/archivis-core/Cargo.toml     crates/archivis-core/Cargo.toml
COPY crates/archivis-db/Cargo.toml       crates/archivis-db/Cargo.toml
COPY crates/archivis-formats/Cargo.toml  crates/archivis-formats/Cargo.toml
COPY crates/archivis-metadata/Cargo.toml crates/archivis-metadata/Cargo.toml
COPY crates/archivis-tasks/Cargo.toml    crates/archivis-tasks/Cargo.toml
COPY crates/archivis-storage/Cargo.toml  crates/archivis-storage/Cargo.toml
COPY crates/archivis-auth/Cargo.toml     crates/archivis-auth/Cargo.toml
COPY crates/archivis-api/Cargo.toml      crates/archivis-api/Cargo.toml
COPY crates/archivis-server/Cargo.toml   crates/archivis-server/Cargo.toml

# Build dependencies only (cached unless Cargo.toml / Cargo.lock change)
RUN cargo build --release

# -- Application build ---------------------------------------------------------
# Remove stubs and copy real source code + SQLx offline metadata
RUN rm -rf crates/
COPY crates/ crates/
COPY .sqlx/  .sqlx/

# Ensure Cargo detects the source change and recompiles application crates
RUN touch crates/*/src/lib.rs crates/archivis-server/src/main.rs \
 && cargo build --release

# ------------------------------------------------------------------------------
# Stage 3: Minimal runtime image
# ------------------------------------------------------------------------------
FROM alpine:3.21

# ca-certificates — required for outbound HTTPS (metadata fetching, etc.)
# tzdata          — allows TZ environment variable to work correctly
RUN apk add --no-cache ca-certificates tzdata

# Non-root user for security
RUN addgroup -S archivis && adduser -S -G archivis -u 1000 archivis

# Copy the statically linked binary from the backend stage
COPY --from=backend /app/target/release/archivis /usr/local/bin/archivis

# Copy the pre-built frontend assets from the frontend stage
COPY --from=frontend /app/frontend/build /usr/share/archivis/frontend

# Create data directories and set ownership
RUN mkdir -p /data /books && chown -R archivis:archivis /data /books

# Configuration — listen on all interfaces inside the container
ENV ARCHIVIS_LISTEN_ADDRESS=0.0.0.0
ENV ARCHIVIS_PORT=9514
ENV ARCHIVIS_DATA_DIR=/data
ENV ARCHIVIS_BOOK_STORAGE_PATH=/books
ENV ARCHIVIS_FRONTEND_DIR=/usr/share/archivis/frontend

EXPOSE 9514

# Persist database and book storage across container restarts
VOLUME ["/data", "/books"]

USER archivis

ENTRYPOINT ["/usr/local/bin/archivis"]
