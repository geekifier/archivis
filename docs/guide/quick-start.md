# Quick Start

## Prerequisites

Archivis runs natively on any modern operating system (Linux or macOS), with fist-class container support making it possible to run almost anywhere, and on minimal hardware.

Pre-built arm64 and x86_64 binaries are provided for Linux and macOS.

## Running Archivis

### From a Release Binary

Download the latest release and run it:

```bash
./archivis
```

Archivis listens on `http://localhost:9514` by default.

### With Docker Compose

```yaml
services:
  archivis:
    image: ghcr.io/geekifier/archivis:latest
    environment:
      TZ: Australia/Lord_Howe
    ports:
      - "9514:9514"
    volumes:
      - archivis-data:/data

volumes:
  archivis-data:
```

```bash
docker compose up -d
```

## Initial Admin Setup

When Archivis starts with an empty database, it redirects to the setup page. The first user you register automatically receives the **Admin** role.

1. Navigate to your Archivis instance (default: `http://localhost:9514`).
2. You will be redirected to `/setup`.
3. Enter a username, password (minimum 8 characters), and optional email.
4. Click **Register** — you are now the admin.

For automated or headless deployments, you can skip the setup wizard and [bootstrap the admin via environment variables](/guide/authentication#automated-admin-bootstrap).

## Creating Additional Users

Only admins can create new users.

1. Go to **Settings** (sidebar, admin-only).
2. In the **Users** section, click **Add User**.
3. Fill in:
   - **Username** (required, must be unique)
   - **Password** (minimum 8 characters)
   - **Email** (optional)
   - **Role**: `admin` or `user`
4. Click **Create**.

## Next Steps

- [Authentication](/guide/authentication) — roles, user management, reverse proxy auth
- [Deployment](/guide/deployment) — Docker Compose and Kubernetes examples
