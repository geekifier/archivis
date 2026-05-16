# Kobo Sync

Pair a Kobo eReader with Archivis and let the device pull selected books over the
Kobo sync protocol. Files are converted to KEPUB on demand, so they appear with
proper page numbers and reading-time estimates on the device.

## How it works

- Archivis exposes the Kobo sync API at `/kobo/{token}` on the same host as the
  web UI. Each paired device gets its own opaque token.
- When the device syncs, it receives entitlements for every book you opted in,
  pulls each one as a KEPUB, and shows it in the home library.
- Selection is per-user. Devices belong to the Archivis user that paired them.

## Kobo device setup

1. **Configure the public base URL.** Open _Settings → Public base URL_ and set
   the externally reachable origin Archivis runs on (e.g.
   `https://books.example.com`). Pairing fails until this is set, because the
   device needs an absolute URL.
2. **Pair the device in Archivis.** Go to _Kobo Sync_ in the sidebar, click
   _Pair device_, and copy the API endpoint shown — the token is displayed only
   once.
3. **Point the Kobo at Archivis.** Connect the Kobo to a computer over USB. On
   the mounted volume, open `.kobo/Kobo/Kobo eReader.conf` in a plain-text
   editor — the `.kobo` directory is hidden, so enable hidden-file display
   first. Find the `[OneStoreServices]` section and replace the
   `api_endpoint` value with the URL from step 2:

   ```ini
   [OneStoreServices]
   api_endpoint=https://books.example.com/kobo/<token>
   ```

   Save the file, safely eject the Kobo, and trigger a sync from the home
   screen. No firmware modification, NickelMenu, or sideloading is required —
   only the config edit. Use HTTPS: the token is part of the URL and is as
   sensitive as a password.

To remove a device entirely, revoke it from _Kobo Sync_; it can no longer
authenticate, even with the original token.

## Running behind a reverse proxy

The Kobo sync API is reached at `/kobo/{token}/...` and authenticates via a token within the URL. If Archivis sits behind a ForwardAuth-style reverse proxy (Authelia, Authentik, oauth2-proxy), you must configure the proxy to bypass authentication for `/kobo/*`, or the Kobo device's sync request will be redirected to a login page and fail. See [Authentication Bypass Paths](/guide/authentication#authentication-bypass-paths) for the authoritative list and per-proxy examples.

## Sync books to Kobo

On any book detail page in Archivis, flip the _Sync to Kobo_ toggle. The next
time the device syncs, the book downloads as a KEPUB.

To stop syncing a book, flip the toggle off — the device removes it on the next
sync.

## Current limitations

The first iteration intentionally keeps the scope small:

- Only EPUB sources are synced. Books without an EPUB cannot be opted in.
- Reading position, bookmarks, annotations, shelves, and collections do not sync.
- The Kobo store is not proxied — only books from your Archivis library appear.
- One file per book is delivered. The UI does not yet expose a per-file picker;
  the backend chooses the oldest EPUB deterministically.
- Syncs are pull-only. There is no background push; books appear on the device
  the next time it syncs on its own schedule.
