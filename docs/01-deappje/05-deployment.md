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

CPU limits are bounded too (2026-10-29, the first preview deploy): the host namespace quota
grants the environment's CPU times 1.5 as burst, 3000m here, beside the vCluster control plane.
The scaffold's shape now renders app 500m per replica, Postgres 1000m, Kratos 250m and 250m per
migration Job, which leaves room for a rolling update's surge pod or a wave-0 Job; the 2 x 1000m
it rendered before did not admit Kratos at all. Raising a limit means raising `cpu` in
`.basable/config.yaml` first.

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

A second workflow, `.github/workflows/apps.yaml`, builds the two apps (below). It is not the
scaffold's and the platform never waits for it: the deploy follows `build.yaml`'s run alone, so a
red app build never blocks a server deployment, and `build.yaml`'s Debian container, which has
no Android SDK and no Xcode, never sees an app target.

## Deviations from the standard shape

1. **`replicas: 1`.** The scaffold renders one `k8s/` for both environments, so the count is the
   same in both: one until live has users (cost on preview, quota headroom on both; the first
   deploy, 2026-10-29). Nothing in the code depends on the count, and the frameworks are
   multi-replica safe, so raising it is one line.
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
   Done 2026-10-30: `code` and `passkey` are off, the courier's secret reference and the
   schema's credential hints are gone (the courier block stays as the placeholder Kratos's
   configuration requires), and the `oidc` method with the Basable provider, the
   `oidc.basable.jsonnet` mapper (verified email or refused) and the two callback paths under
   `/.ory/` the platform registers are what is left. The `kratos` database, the admin API for
   `delete_identity` and the session check in the api crate are unchanged.

## The apps (`core/`, `ios/`, `android/`)

Round 23 (2026-10-29). The apps live in this repository, beside `frontend/`, as targets of the
one `MODULE.bazel`; nothing pins a commit of anything. The scaffold has no notion of them (the
manifest has no `apps`, `check_scaffold` is structural and looks at nanoservice directories
only), so the implementing session adds them by hand and the scaffold's own files are not
touched: `MODULE.bazel` gains modules outside the `basable:` markers, `build.yaml` and
`verify.sh` stay as rendered.

- **`core/`**: the Rust client core, a `rust_library` with `uniffi` (proc-macro exports, no UDL)
  and `wasm-bindgen` behind features. Host build: its conformance tests (`03-testing.md`) and a
  `rust_shared_library` the binding generator reads. Android: a `rust_shared_library` per ABI
  (`aarch64-linux-android`, `x86_64-linux-android` for the emulator) through `rules_android_ndk`.
  iOS: a `rust_static_library` for `aarch64-apple-ios` and `aarch64-apple-ios-sim`. Web:
  `wasm32-unknown-unknown` through `rules_rust`'s wasm-bindgen rules, the target
  `frontend/react` already depends on. All five triples are `extra_target_triples` of the one
  Rust toolchain. `uniffi-bindgen` is a `rust_binary` of the `uniffi` crate's `cli` feature
  (a `crate.annotation` with `gen_binaries`, like the messenger generator), and one genrule per
  language runs `generate --library` on the host library into `core/bindings/{swift,kotlin}`.
- **`android/`**: Kotlin and Jetpack Compose. `rules_kotlin`, `rules_android`,
  `rules_jvm_external` for the Maven graph (Compose, `net.java.dev.jna:jna@aar`, which the
  UniFFI Kotlin bindings load the core through). A `kt_android_library` of the generated
  bindings plus the per-ABI core libraries, a `kt_android_library` of the app, an
  `android_binary` `//android:app`, debug-signed. Login, push, media, the calendar mirror and
  pairing as `01-architecture.md` divides them.
- **`ios/`**: Swift and SwiftUI. `rules_swift`, `rules_apple`, `apple_support`. A `cc_library`
  of the generated C header and module map with the core static library, a `swift_library` of
  the generated Swift bindings, a `swift_library` of the app, an `ios_application` `//ios:app`
  for the simulator; a device build needs a provisioning profile and is `later` with signing.
  `ios_unit_test` for the view models.
- **The flag**: a `bool_flag` `//:apps` (default off) with a `config_setting`; every target under
  `ios/` and `android/` and every non-host build of `core/` carries
  `target_compatible_with = select({"//:apps_on": [], "//conditions:default":
  ["@platforms//:incompatible"]})`, the iOS ones `["@platforms//os:macos"]` on the on-side as
  well. `bazel test //...` skips incompatible targets, so the scaffold's `verify.sh`,
  `build.yaml` and a laptop's everyday run build the server, the core's host tests and the web
  client only; `bazel test --//:apps //android/...` (Linux or macOS) and `--//:apps //ios/...`
  (macOS) are the app runs. The first implementation step verifies, in the `build.yaml`
  container, that `//...` stays green with the app modules merely loaded: no Xcode, no
  `ANDROID_HOME`.
- **`apps.yaml`**: on pushes to `dev` and `main` touching `core/**`, `ios/**`, `android/**`,
  `proto/**`, `Cargo.*` or `MODULE.bazel*`. Job `android` on `ubuntu-latest` without a job
  container (the runner image carries the Android SDK and NDK; `ANDROID_HOME` and
  `ANDROID_NDK_HOME` are set): `bazel test --config=ci --//:apps //android/...`, then the debug
  APK as an artifact. Job `ios` on `macos-latest` (Xcode present): the same for `//ios/...`, the
  simulator `.app` as an artifact. Both jobs cache like `build.yaml`, one pair per OS. The
  repository is public, so neither runner costs minutes; the platform's native-apps plan keeps
  macOS off by default for private repositories, which does not apply here. `buf breaking`
  runs in this workflow without `|| true`: an installed app lives for months, so the API
  changes additively once a build has shipped; `build.yaml`'s own step stays the scaffold's.
- **Base URL**: `environments.bzl` holds the preview and live platform hostnames (from
  `get_project`); a `string_flag` `//:environment` (`preview` default, `live`) picks one at
  build time, and `apps.yaml` passes `live` on `main`. A custom domain later changes nothing an
  installed app relies on, since the platform hostnames keep serving. The Kratos native flow's
  return URL is the app's own scheme, registered in `Info.plist` and the Android manifest.
- **Store setup** (Apple developer account, App Store Connect record, Play console app, the
  signing material as GitHub secrets, TestFlight and internal-track upload on `main`) is `later`
  and by hand; `android/README.md` and `ios/README.md` list the steps. Apple's guideline 4.8
  requires Sign in with Apple once an app offers a third-party login, and Sign in with Basable
  is one; that is a store question for the submission, not a plan change.

## Out of the scaffold, "later, via the agent"

Database backups and the public-bucket website move. The apps are in this repository (above)
but equally outside the scaffold.
