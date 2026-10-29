# Deployment

## Sizing (free-tier quota per environment: 2000m CPU, 4096Mi memory, 10Gi storage)

| Workload | Replicas | CPU req | Memory | Storage |
|---|---|---|---|---|
| app (the binary) | 1 preview / 2 live | 100m | 256Mi | |
| frontend (nginx-unprivileged) | 1 | 50m | 64Mi | |
| CNPG cluster | 1 instance | 250m | 512Mi | 2Gi |
| CNPG operator | 1 | 100m | 200Mi | |
| kratos public + admin | 1 each | 50m | 128Mi each | |
| kratos migrate, app migrate | Jobs | 50m | 128Mi | |

Preview total ≈ 750m / 1.5Gi; live ≈ 850m / 1.75Gi. Both inside the free quota. The organisation
has a payment method, so growth is possible, but nothing here needs it.

## `.basable/config.yaml`

```yaml
apiVersion: basable.com/v1
kind: TenantConfig
spec:
  environments:
    preview:
      virtual-cluster: { cpu: "2000m", memory: "4Gi", storage: "10Gi", maxPods: 30 }
      objectStorage: {}
    live:
      virtual-cluster: { cpu: "2000m", memory: "4Gi", storage: "10Gi", maxPods: 30 }
      objectStorage: {}
```

`objectStorage` in both: `storage` needs the private bucket for blobs. The `object-storage`
Secret reaches the app container through `envFrom`. Nothing goes in the public bucket until the
web client moves there (below).

## Secrets

| Key | Source | Used by |
|---|---|---|
| `app-db-app` / `uri` | CNPG-minted | app, migrate Job |
| `kratos-db-app` / `uri` | CNPG-minted | kratos |
| `object-storage` (whole Secret) | platform-minted | app |
| `basable-login` / `BASABLE_LOGIN_ISSUER`, `BASABLE_LOGIN_CLIENT_ID`, `BASABLE_LOGIN_CLIENT_SECRET` | platform-written, per environment | kratos, the `oidc` provider list (`SELFSERVICE_METHODS_OIDC_CONFIG_PROVIDERS` from three `secretKeyRef` env vars); never declared in `k8s/`, never asked of the user |
| `app-secrets` / `APNS_KEY_P8`, `APNS_KEY_ID`, `APNS_TEAM_ID`, `APNS_BUNDLE_ID` | **user** | `device`, from the Apple Developer account's push key |
| `app-secrets` / `FCM_SERVICE_ACCOUNT_JSON` | **user** | `device`, a Firebase service account with the Messaging role |
| kratos cookie/cipher secrets | generated in-cluster | kratos; the 32-character cipher secret is what the `oidc` method needs |
| Kratos admin API | in-cluster service address, no secret | `group`, for `delete_identity` |

Each user secret is its own non-optional `secretKeyRef`; the first preview deploy waits at the
secrets step until the user adds them, per environment. The push secrets are the only user
secrets left (round 17 removed SMTP): the app starts without them and `device` logs and skips
sends until they exist. That is the one place the implementation deviates from "non-optional":
the app reads them at send time from the environment, which the deploy still requires to be
present. Decision for the implementing session: require all four from day one (simplest) and
tell the user where to get them.

## Database

One CNPG cluster `app-db`, `instances: 1`, no storage class named, the pinned operator release
the platform's own apps use. Schemas `group`, `device`, `relay`, `storage`, each with its own
role (`basable-db`), plus the processing-object envelope schema. Separate `kratos` database on the
same cluster. Migrations are dbmate files run by the migrate Job in wave -1. No nanoservice owns
the accounting ledger (`owns.ledger`), so there is no restatement at boot.

**Backups: later.** The private bucket exists from the first deploy, so adding
`backup.barmanObjectStore` with `retentionPolicy: "14d"` and a daily `ScheduledBackup` is a
small change the agent makes before live has real users. Until then preview data is disposable.

## CI / registry

GitHub (`deappje/deappje`), images to `registry.basable.com/abandon-above-toss/deappje`. The
scaffold renders the platform's artifact workflow (lint → Bazel build and push → render `k8s/`
with digests → upload `rendered-manifests`) with `paths-ignore: ["docs/**"]`.

## Deviations from the standard shape

1. **`replicas: 1` in preview.** Cost. Live runs 2. Nothing in the code depends on the count.
2. **Web client from the public bucket, later.** The scaffold serves `frontend/react` from an
   nginx-unprivileged container. Once the environment has object storage (it does, from the first
   deploy), the agent switches to `objectStorage: { website: true }`, a `site-upload` Job in wave
   -1, and removes the frontend Deployment, Service and the `/` route from the HTTPRoute; the
   app's route keeps only `/api` and `/.ory`. Saves 50m/64Mi and keeps the site up while preview
   sleeps. Done as a follow-up in a default session, not in the scaffold.
3. **No Kratos UI container.** Login screens are the clients' own (native and React) against
   Kratos's public API: one "Sign in with Basable" button that starts the `oidc` flow, the
   browser flow on the web, the native flow on the phones. No email or passkey screens. The
   scaffold's default is the same; noted so nobody adds one.
4. **Sign in with Basable is the only method.** The scaffold's Kratos ConfigMap enables email
   code, passkeys and Basable together, with an optional `SMTP_CONNECTION_URI` for the courier.
   The implementing session turns `code` and `passkey` off, removes the courier reference, and
   keeps the `oidc` method with the Basable provider, the `oidc.basable.jsonnet` mapper (verified
   email or refused) and the two callback paths under `/.ory/` the platform registers. The
   `kratos` database, the admin API for `delete_identity` and the session check in the api crate
   are unchanged.

## Out of the scaffold, "later, via the agent"

Database backups, the public-bucket website move, and the mobile apps themselves, which live in
their own repositories and build the `core/` crate from a pinned commit of this repository with
UniFFI. The web client in this repository compiles the same crate to wasm in the build workflow.
