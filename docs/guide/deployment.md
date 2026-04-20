# Deployment

## Docker Compose

```yaml
services:
  archivis:
    image: ghcr.io/geekifier/archivis:latest
    environment:
      TZ: Australia/Lord_Howe
      ARCHIVIS_PUBLIC_BASE_URL: https://books.example.com
    ports:
      - "9514:9514"
    volumes:
      - archivis-data:/data
      - archivis-books:/books

volumes:
  archivis-data:
  archivis-books:
```

See [Automated Admin Bootstrap](/guide/authentication#automated-admin-bootstrap) for all bootstrap environment variables, including Docker secrets support via `ARCHIVIS_ADMIN_PASSWORD_FILE`.

## Kubernetes

This example uses the [bjw-s app-template](https://github.com/bjw-s-labs/helm-charts/tree/main/charts/other/app-template) Helm chart with Flux, but the same concepts apply to any Kubernetes deployment method.

### HelmRelease Example

```yaml
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: archivis
spec:
  interval: 1h
  chartRef:
    kind: OCIRepository
    name: app-template
  values:
    controllers:
      archivis:
        replicas: 1
        strategy: Recreate
        containers:
          app:
            image:
              repository: ghcr.io/geekifier/archivis
              tag: latest
            env:
              TZ: Australia/Lord_Howe
              ARCHIVIS_PUBLIC_BASE_URL: https://books.example.com
            envFrom:
              - secretRef:
                  name: archivis-secret
            probes:
              startup:
                enabled: true
                custom: true
                spec:
                  httpGet:
                    path: /health/live
                    port: &port 9514
                  periodSeconds: 2
                  failureThreshold: 15
              liveness:
                enabled: true
                custom: true
                spec:
                  httpGet:
                    path: /health/live
                    port: *port
                  periodSeconds: 10
                  failureThreshold: 3
              readiness:
                enabled: true
                custom: true
                spec:
                  httpGet:
                    path: /health/ready
                    port: *port
                  periodSeconds: 10
                  failureThreshold: 3
            resources:
              requests:
                cpu: 50m
                memory: 128Mi
              limits:
                memory: 1024Mi
            securityContext:
              allowPrivilegeEscalation: false
    defaultPodOptions:
      securityContext:
        runAsNonRoot: true
        runAsUser: 1000
        runAsGroup: 1000
        fsGroup: 1000
        fsGroupChangePolicy: OnRootMismatch
    service:
      app:
        controller: archivis
        ports:
          http:
            port: *port
    persistence:
      data:
        enabled: true
        type: persistentVolumeClaim
        size: 4Gi
        accessMode: ReadWriteOnce
        globalMounts:
          - path: /data
      books:
        enabled: true
        type: persistentVolumeClaim
        size: 16Gi
        accessMode: ReadWriteOnce
        globalMounts:
          - path: /books
```

### Key Points

- **Secrets**: Store sensitive values like `ARCHIVIS_ADMIN_PASSWORD` in a Kubernetes Secret and reference it via `envFrom`. See [Automated Admin Bootstrap](/guide/authentication#automated-admin-bootstrap).
- **Health probes**: Archivis exposes `/health/live` (liveness) and `/health/ready` (readiness) endpoints on port 9514.
- **Storage**: Use separate PVCs for `/data` (database + config) and `/books` (ebook storage) so they can be sized and backed up independently.
- **Security context**: Archivis runs as non-root (UID 1000) with no privilege escalation.
- **Public URL**: Set `ARCHIVIS_PUBLIC_BASE_URL` to the stable externally reachable Archivis URL. Features that emit absolute links outside request context depend on it.

### With Reverse Proxy Auth

If running behind an auth proxy (Authelia, Authentik, etc.), add the proxy auth env vars to the container and configure the ingress to forward authentication headers. Set `ARCHIVIS_AUTH__PROXY__TRUSTED_PROXIES` to your cluster's pod CIDR. See [Reverse Proxy Authentication](/guide/authentication#reverse-proxy-authentication) for the full configuration reference.

```yaml
# Add to containers.app.env:
env:
  TZ: Australia/Lord_Howe
  ARCHIVIS_PUBLIC_BASE_URL: https://books.example.com
  ARCHIVIS_AUTH__PROXY__ENABLED: "true"
  ARCHIVIS_AUTH__PROXY__TRUSTED_PROXIES: "[100.64.0.0/16]"
  # Match these to the auth response headers configured in your auth proxy
  ARCHIVIS_AUTH__PROXY__USER_HEADER: Remote-User
  ARCHIVIS_AUTH__PROXY__EMAIL_HEADER: Remote-Email
  ARCHIVIS_AUTH__PROXY__GROUPS_HEADER: Remote-Groups

# Add to values:
ingress:
  app:
    enabled: true
    className: internal
    annotations:
      nginx.ingress.kubernetes.io/auth-method: "GET"
      nginx.ingress.kubernetes.io/auth-url: "http://authelia.auth.svc.cluster.local:9091/api/authz/auth-request"
      nginx.ingress.kubernetes.io/auth-signin: "https://auth.example.com?rm=$request_method"
      nginx.ingress.kubernetes.io/auth-response-headers: "Remote-User,Remote-Name,Remote-Groups,Remote-Email"
    hosts:
      - host: books.example.com
        paths:
          - path: /
            service:
              identifier: app
              port: http
```
