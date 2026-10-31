# ios

The iOS app: Swift and SwiftUI over the client core
(`docs/01-deappje/01-architecture.md`, "The shells"; `05-deployment.md`,
"The apps"). The core (`core/`, Rust) owns the store, sync, the keys and
every object's shape; this shell renders its state and does what only the
platform can: the sign-in in the system browser, the Keychain, the device
key, the files, URLSession, push.

## Layout

| Path | What |
|---|---|
| `BUILD.bazel`, `Info.plist`, `app.entitlements` | `//ios:app`, the `ios_application` for the simulator: bundle id `nl.deappje.app`, iOS 17.0 and up, the `deappje://` URL scheme, `remote-notification` background mode. The entitlements name the application identifier under a placeholder team prefix, which the simulator's Keychain requires of a process (`-34018` without it). |
| `defs.bzl` | `IOS_ONLY` (every target here is iOS-only and behind `--//:apps`), `deployment_swift` (the environment's base URL as Swift), `host_files` (host-generated bindings as an iOS build's inputs), `ios_test` (a testonly module and its `ios_unit_test`). |
| `core/` | `DeappjeCore`: the UniFFI Swift bindings `//core:bindings_swift` generates from the host build, over the C header (module `deappje_coreFFI`) and `//core:core_ios`, the Rust static library for whichever iOS platform rules_apple's transition selects. |
| `shell/` | `DeappjeShell`: the six seams the core takes from the platform (`core/src/shell.rs`, `transport.rs`). |
| `application/` | `DeappjeApp`: `AppModel` and `TimelineModel` (observable, `@MainActor`) over the one `Core`, `WakeUp` (a push's reason to the core call it names), the SwiftUI screens, the app delegate for push. `Deployment.swift` is generated. |
| `tests/` | One `ios_unit_test` per seam, the end-to-end run over the core's in-memory server, the models over the same; the two Keychain tests run inside `tests/host`, an empty app with the same kind of entitlements, one at a time, under rules_apple's legacy runner (see below). |

## The seams

| Seam | Implementation |
|---|---|
| `Random` | `SecureRandom`: `SecRandomCopyBytes`. |
| `Clock` | `SystemClock`: `Date`, milliseconds since the epoch. |
| `Secrets` | `KeychainSecrets`: generic-password items, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, service `nl.deappje.app`, one item per name under the `secret.` account prefix. |
| `DeviceKey` | `KeychainDeviceKey`: X25519 with CryptoKit (`Curve25519.KeyAgreement`). The Secure Enclave does P-256 only and cannot hold an X25519 key, so the private half is a Keychain item (this device only, after first unlock) loaded for the length of one Diffie-Hellman step; the object holds the public half. `SoftwareDeviceKey` is the same over a key in memory (tests). |
| `Storage` | `FileStorage`: one file per key under `Application Support/deappje/store`, atomic writes, `completeFileProtectionUntilFirstUserAuthentication`, key to file name by percent-encoding. |
| `Transport` | `URLSessionTransport`: an ephemeral URLSession, no cookies, no cache; response header names lowercased; a network failure is `TransportError.Network`. |

## Build, test, run

Everything here is behind `--//:apps` and needs Xcode (the simulator SDK
and an iOS simulator runtime; `xcodebuild -downloadPlatform iOS` fetches
one). With the flag off, `bazel test //...` skips all of it.

```sh
# The simulator app (preview environment); bazel-bin/ios/app.ipa holds deappje.app
bazel build --//:apps //ios:app
# Against the live environment (what apps.yaml builds on main)
bazel build --//:apps --//:environment=live //ios:app

# Every test, on the simulator
bazel test --//:apps //ios/...
# One of them; --ios_simulator_device / --ios_simulator_version pick the simulator
bazel test --//:apps //ios/tests:end_to_end_test

# Install and launch on a booted simulator
xcrun simctl boot "iPhone 17 Pro"
unzip -o bazel-bin/ios/app.ipa -d /tmp/deappje-app
xcrun simctl install booted /tmp/deappje-app/Payload/deappje.app
xcrun simctl launch booted nl.deappje.app
```

The two Keychain tests (`secrets_test`, `device_key_test`) need a process
with an application identifier, so they run inside `tests/host`. They use
rules_apple's legacy runner (Google's xctestrunner, `tests/BUILD.bazel`
`keychain_runner`), which installs the host for real: the default
xctestrun runner has `xcodebuild` register the host at its own path, and
the simulator's securityd then sees none of its entitlements (`-34018`,
"Client has neither application-identifier nor keychain-access-groups
entitlements"). Signing the entitlements into the ad-hoc signature is no
way out: AMFI refuses to launch an ad-hoc binary with restricted
entitlements. The runner's device type (`iPhone 17`) must exist in the
installed runtime; `--ios_simulator_device` and `--ios_simulator_version`
steer the other tests.

If the installed simulator runtime is newer than Xcode's SDK (an
`xcodebuild -downloadPlatform iOS` fetches the newest), the default runner
finds "no matching runtimes": `xcrun simctl runtime match set
iphoneos<sdk> <runtime build>` maps the SDK to it.

The base URL comes from `environments.bzl` through the `//:environment`
flag (`preview` by default, `live`), never from a Swift source: `ios/application`
generates `Deployment.swift` from it.

Sign in opens Basable's login in an `ASWebAuthenticationSession`; it comes
back to `deappje://login?code=...`, and the core exchanges the code for the
session token (`core/src/kratos.rs`). After sign-in the app registers the
device (`ios`), asks for a push token (the simulator gives none, which is
not an error) and syncs: at once on every activation, then every 30 s while
active, and on a silent push (`core/AGENTS.md`, "Wake-ups": `head`, `keys`,
`rotate`, `history` sync; `key_request` refreshes the approvals;
`history_ready` is handed to the core, then a sync). The approvals list is
refreshed on every activation as well, since no sync lists it.

## Still manual: the store

As `05-deployment.md` leaves it, the store setup is later and by hand:

- An Apple Developer account and an App Store Connect record for
  `nl.deappje.app`.
- Signing for a device build: a development or distribution certificate and
  a provisioning profile (`ios_application`'s `provisioning_profile`), and
  `app.entitlements` rewritten for it: `$(AppIdentifierPrefix)` in place of
  the placeholder team prefix (rules_apple substitutes it from the profile)
  and `aps-environment` added. The simulator build here is ad-hoc signed. The APNs key and the bundle id go
  to `device`'s push secrets (`02-device.md`, "Implementation record").
- The signing material as GitHub secrets, and a TestFlight upload on
  `main` in `apps.yaml`.
- Sign in with Apple: App Store guideline 4.8 requires it once an app
  offers a third-party login, and Sign in with Basable is one. That is a
  store question for the submission (Kratos gets an Apple OIDC provider,
  the app a second button), not a change to this shell's shape.
- An app icon and a launch screen beyond the empty `UILaunchScreen`.

## What is not here yet

Media (the picker, the camera, the resize), the system calendar mirror,
QR pairing, export, the richer timeline (photos, comments, reactions), the
other object types' screens, a device build.
