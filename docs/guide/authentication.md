# Authentication

## Roles and Permissions

| Role | Permissions |
|------|-------------|
| **Admin** | Full access: library management, import, settings, user management |
| **User** | Library access: browse, read, import. No access to settings or user management |

Only admins can see the **Settings** page in the sidebar.

## User Management

The user management table (**Settings > Users**) shows all users with their username, email, role, status (active/inactive), and creation date.

### Available Actions

- **Edit** (pencil icon): Change username, email, or role.
- **Reset Password** (key icon): Set a new password for the user (admin only).
- **Deactivate/Activate** (circle icon): Toggle the user's active status.

### Restrictions

- You cannot deactivate yourself.
- You cannot deactivate the last active admin — there must always be at least one active admin.
- Deactivating a user immediately invalidates all their sessions (they are logged out).

### Deactivation vs Deletion

Archivis uses **soft-delete**. The DELETE endpoint sets `is_active=false` rather than removing the user record. This preserves audit history and allows reactivation later.

## Password Management

### Changing Your Own Password

Any authenticated user can change their own password:

1. Click the **key icon** in the top-right header bar (next to your username).
2. Enter your **current password** and the **new password** (minimum 8 characters).
3. Click **Change Password**.

After changing your password, all your existing sessions are invalidated — you will need to log in again on all devices.

### Admin Password Reset

Admins can reset any user's password without knowing their current password:

1. Go to **Settings > Users**.
2. Click the **key icon** next to the target user.
3. Enter the new password (minimum 8 characters).
4. Click **Reset Password**.

This invalidates all of the target user's sessions.

## Automated Admin Bootstrap

For automated deployments (Docker, CI, headless environments), you can create the initial admin user via environment variables instead of the setup wizard.

| Variable                       | Description                                                                                              |
| ------------------------------ | -------------------------------------------------------------------------------------------------------- |
| `ARCHIVIS_ADMIN_USERNAME`      | Username for the bootstrap admin (triggers bootstrap)                                                    |
| `ARCHIVIS_ADMIN_PASSWORD`      | Password for the bootstrap admin (min 8 characters)                                                      |
| `ARCHIVIS_ADMIN_PASSWORD_FILE` | Path to a file containing the password (Docker secrets; takes precedence over `ARCHIVIS_ADMIN_PASSWORD`) |
| `ARCHIVIS_ADMIN_EMAIL`         | Optional email address for the bootstrap admin                                                           |

Bootstrap only runs when no users exist in the database (first boot). These are one-time variables — they are not part of the config file or settings API.

::: tip
If an admin already exists and bootstrap env vars are set, a warning is logged suggesting you remove them for security. If `ARCHIVIS_ADMIN_USERNAME` is set but no password is provided, Archivis exits with an error.
:::

### Example: Docker Compose with Bootstrap

```yaml
services:
  archivis:
    image: ghcr.io/geekifier/archivis:latest
    environment:
      TZ: Australia/Lord_Howe
      ARCHIVIS_ADMIN_USERNAME: admin
      ARCHIVIS_ADMIN_PASSWORD: changeme123
    ports:
      - "9514:9514"
    volumes:
      - archivis-data:/data

volumes:
  archivis-data:
```

## Reverse Proxy Authentication

Archivis supports authentication via a reverse proxy that implements the ForwardAuth pattern (used by Authelia, Authentik, and similar identity providers). When enabled, Archivis trusts user identity headers set by the proxy and automatically creates or updates local user records.

### Configuration

At minimum, enable proxy auth and specify which proxy IPs to trust:

```bash
ARCHIVIS_AUTH__PROXY__ENABLED=true
ARCHIVIS_AUTH__PROXY__TRUSTED_PROXIES="[192.168.100.200/30]"
```

All proxy auth settings require a **server restart** to take effect.

#### All Options

| Variable | Default | Description |
|----------|---------|-------------|
| `ARCHIVIS_AUTH__PROXY__ENABLED` | `false` | Enable reverse proxy authentication |
| `ARCHIVIS_AUTH__PROXY__TRUSTED_PROXIES` | (empty) | Array of trusted proxy IP addresses or CIDR ranges (e.g. `[10.0.0.1, 172.18.0.0/16]`) |
| `ARCHIVIS_AUTH__PROXY__USER_HEADER` | `X-Forwarded-User` | Header containing the authenticated username |
| `ARCHIVIS_AUTH__PROXY__EMAIL_HEADER` | `X-Forwarded-Email` | Header containing the user's email |
| `ARCHIVIS_AUTH__PROXY__GROUPS_HEADER` | `X-Forwarded-Groups` | Header containing comma-separated group names (reserved for future role mapping) |

These can also be set in `config.toml`:

```toml
[auth.proxy]
enabled = true
trusted_proxies = ["192.168.100.200/30"]
user_header = "X-Forwarded-User"
email_header = "X-Forwarded-Email"
groups_header = "X-Forwarded-Groups"
```

### How It Works

1. The proxy authenticates the user (SSO, LDAP, etc.) and sets headers on the request forwarded to Archivis.
2. Archivis checks whether the request originates from a trusted proxy IP.
3. If trusted, it reads the username from the configured header.
4. If the user does not exist, Archivis **auto-creates** them with the `User` role.
5. If the user already exists, their email is updated if it changed.

Users created via proxy auth receive the `User` role. To grant Admin access, use the admin UI or bootstrap an admin via environment variables.

::: warning
Users created via proxy auth cannot log in with a local password. They must always authenticate through the proxy.
:::

### Trusted Proxy CIDR Configuration

The `trusted_proxies` field accepts:

- **Individual IPs**: `10.0.0.1`, `::1`
- **CIDR ranges**: `172.16.0.0/12`, `fd00::/8`
- **Multiple entries** (array in env vars and TOML): `[10.0.0.1, 172.18.0.0/16]`

**Guidelines:**

- Use the narrowest CIDR range that covers your proxy. For Docker, this is typically the Docker network subnet (e.g., `172.18.0.0/16`).
- For a single known proxy IP, use the exact address (e.g., `10.0.0.5`).
- Both IPv4 and IPv6 are supported.
- An empty list means no proxy is trusted (proxy auth is effectively disabled).

### Example: Authelia with Docker Compose

```yaml
services:
  archivis:
    image: ghcr.io/geekifier/archivis:latest
    environment:
      TZ: Australia/Lord_Howe
      ARCHIVIS_ADMIN_USERNAME: admin
      ARCHIVIS_ADMIN_PASSWORD: changeme123
      ARCHIVIS_AUTH__PROXY__ENABLED: "true"
      ARCHIVIS_AUTH__PROXY__TRUSTED_PROXIES: "[172.18.0.0/16]"
    networks:
      - proxy

  authelia:
    image: authelia/authelia:latest
    # ... Authelia configuration ...
    networks:
      - proxy

networks:
  proxy:
    driver: bridge
    ipam:
      config:
        - subnet: 172.18.0.0/16
```

Authelia sets the following headers by default (matching Archivis defaults):

- `Remote-User` — set `ARCHIVIS_AUTH__PROXY__USER_HEADER=Remote-User`
- `Remote-Email` — set `ARCHIVIS_AUTH__PROXY__EMAIL_HEADER=Remote-Email`
- `Remote-Groups` — set `ARCHIVIS_AUTH__PROXY__GROUPS_HEADER=Remote-Groups`

### Example: Caddy with forward_auth

```
archivis.example.com {
    forward_auth authelia:9091 {
        uri /api/authz/forward-auth
        copy_headers Remote-User Remote-Email Remote-Groups
    }
    reverse_proxy archivis:9514
}
```

Set the corresponding Archivis env vars to match the Caddy header names:

```bash
ARCHIVIS_AUTH__PROXY__USER_HEADER=Remote-User
ARCHIVIS_AUTH__PROXY__EMAIL_HEADER=Remote-Email
ARCHIVIS_AUTH__PROXY__GROUPS_HEADER=Remote-Groups
```

### Example: Traefik with ForwardAuth Middleware

```yaml
# docker-compose labels for the archivis service
labels:
  - "traefik.http.routers.archivis.middlewares=authelia@docker"
  - "traefik.http.middlewares.authelia.forwardAuth.address=http://authelia:9091/api/authz/forward-auth"
  - "traefik.http.middlewares.authelia.forwardAuth.authResponseHeaders=Remote-User,Remote-Email,Remote-Groups"
```

### Example: nginx with auth_request

```nginx
server {
    listen 443 ssl;
    server_name archivis.example.com;

    location /authelia {
        internal;
        proxy_pass http://authelia:9091/api/authz/auth-request;
        proxy_pass_request_body off;
        proxy_set_header Content-Length "";
        proxy_set_header X-Original-URL $scheme://$http_host$request_uri;
    }

    location / {
        auth_request /authelia;
        auth_request_set $user $upstream_http_remote_user;
        auth_request_set $email $upstream_http_remote_email;
        auth_request_set $groups $upstream_http_remote_groups;

        proxy_set_header Remote-User $user;
        proxy_set_header Remote-Email $email;
        proxy_set_header Remote-Groups $groups;
        proxy_pass http://archivis:9514;
    }
}
```

### Security Notes

::: danger
**Always restrict `trusted_proxies`** to the specific IP or subnet of your reverse proxy. Never set it to `0.0.0.0/0` — this would allow any client to spoof authentication headers.
:::

- **Never expose Archivis directly** to the internet when proxy auth is enabled. All traffic must pass through the reverse proxy.
- Proxy auth and local auth **coexist**: if a request does not come from a trusted proxy (or lacks the required headers), Archivis falls back to session-based authentication. This means you can still log in locally for initial setup.
- IP validation uses the actual TCP source address, not `X-Forwarded-For`, so proxy header spoofing from untrusted sources is not possible.

## Troubleshooting

### "Missing authentication token" when proxy auth should be active

- Verify `ARCHIVIS_AUTH__PROXY__ENABLED=true` and restart Archivis.
- Check that `trusted_proxies` includes the IP address of your reverse proxy. If using Docker, this is the container's IP on the shared network, not the host IP.
- Ensure your proxy is setting the user header (check with `curl -H "X-Forwarded-User: test" http://archivis:9514/api/auth/me` from within the proxy's network).

### Proxy-created user needs Admin role

Proxy auth always creates users with the `User` role. To promote a proxy user to Admin, use the admin UI (**Settings > Users > Edit**) or bootstrap an admin via `ARCHIVIS_ADMIN_USERNAME` / `ARCHIVIS_ADMIN_PASSWORD` env vars on first boot.

### Cannot log in locally after enabling proxy auth

- Local (session-based) auth still works alongside proxy auth. If you are accessing Archivis directly (not through the proxy), the login page should work normally.
- Use `ARCHIVIS_ADMIN_USERNAME` / `ARCHIVIS_ADMIN_PASSWORD` to bootstrap an admin on first boot, so you don't need to bypass the proxy for initial setup.

### "Cannot deactivate the last admin"

Archivis requires at least one active admin at all times. Create a second admin user before deactivating the current one.

### Password change logs me out everywhere

This is expected behavior. Changing your password (or having an admin reset it) invalidates all existing sessions as a security measure. Log in again with the new password.
