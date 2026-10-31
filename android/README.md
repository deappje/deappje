# The Android app

Kotlin and Jetpack Compose over the shared Rust core, bound through UniFFI
(`docs/01-deappje/05-deployment.md`, "The apps"; `core/AGENTS.md` for the
core and its seams). The shell renders what the core holds and does what
only the platform can: the browser for the sign-in, the Keystore for the
keys, files for the store, OkHttp for the network. It never parses an
object value, never holds a group key and never calls the server itself.

## What is where

| Path | What |
|---|---|
| `BUILD.bazel` | The bindings (compiled for the phone and for the host tests), the seams, the app, the APK, the tests. Every target is behind `--//:apps`. |
| `ndk.bzl` | A module extension around rules_android_ndk's repository: the NDK when `ANDROID_NDK_HOME` is set, an empty repository otherwise, so the server's `bazel test //...` never needs one. |
| `environment.bzl` | `Environment.kt`, generated from the root `environments.bzl` by `--//:environment` (`preview` by default, `live` from apps.yaml on main). The base URL is never typed in Kotlin. |
| `maven_app_install.json`, `maven_host_install.json` | The pinned Maven graphs of the app and of the host tests (`MODULE.bazel`, the `# --- android/ ---` block). |
| `src/main/kotlin/com/deappje/android/shell/` | The six seams in plain Kotlin, no Android API: `SecureRandomSource`, `SystemClock`, `FileStorage`, `EncryptedFileSecrets`, `SoftwareDeviceKey` with `X25519`, `OkHttpTransport`. The same sources are compiled for the host tests. |
| `src/main/kotlin/com/deappje/android/keystore/` | The Android Keystore parts: the AES key the secrets file is sealed under, the X25519 key inside the Keystore (API 33+), and `DeviceKeys`, which decides between them. |
| `src/main/kotlin/com/deappje/android/ui/` | `MainActivity` (the Custom Tab, the `deappje://auth` return, the foreground sync loop), `AppViewModel` (every call into the core), the Compose screens. |
| `src/main/kotlin/com/deappje/android/push/` | The push-token seam; no provider is in the build. |
| `src/test/kotlin/com/deappje/android/` | One `kt_jvm_test` per seam, and `EndToEndTest` over the in-memory server. |

## The seams, as chosen

- **Secrets**: a file under the app's no-backup directory, each value sealed
  with AES-256-GCM under a key in the Android Keystore (alias
  `deappje.secrets`), the name as associated data. Not
  `EncryptedSharedPreferences`: `androidx.security:security-crypto` is
  deprecated as a whole, and this is sixty lines over `javax.crypto`.
- **DeviceKey**: on Android 13 and later an X25519 pair inside the Android
  Keystore (alias `deappje.device`), the Diffie-Hellman step run in the
  Keystore. Where the Keystore cannot (older Android, a KeyMint without
  Curve25519), an X25519 key in software whose secret lives in the Secrets
  store. A key, once minted, is kept for good: the server refuses a changed
  key under a known device id. The sign-in and groups screens say which one
  is in use. The software ladder is RFC 7748 over `BigInteger`: tested
  against the RFC's vectors, not constant-time, and used only where the
  Keystore cannot be.
- **Storage**: one file per key under the no-backup files directory, the
  key's UTF-8 as a URL-safe base64 file name; writes land through a rename.
- **Random**: `SecureRandom`. **Clock**: `System.currentTimeMillis()`.
- **Transport**: OkHttp; the core sets every header, a status is an answer,
  only the network is an error.
- **Push**: `PushTokenSource` with `NoPush`. Firebase needs a
  `google-services.json`, which does not exist yet; a Firebase-backed source
  implements the interface and hands the token to `setPushToken("fcm", ...)`.
  Until then wake-ups are the foreground and the 30-second timer, which the
  core is designed for (`core/AGENTS.md`, "Wake-ups").

## Building

The SDK and NDK come from the environment; the laptops install them with
Homebrew (`android-commandlinetools`), platform `android-35`, build-tools
`35.0.0`, NDK `27.3.13750724`, and apps.yaml pins the same on the runner.

```sh
export ANDROID_HOME=/opt/homebrew/share/android-commandlinetools
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/27.3.13750724

# The debug APK, the core built for arm64-v8a and x86_64 (the platforms in
# .bazelrc's `# --- android/ ---` block), talking to preview:
bazel build --//:apps //android:app
# ... or to live:
bazel build --//:apps --//:environment=live //android:app

# The tests: each seam, and two phones over the in-memory server.
bazel test --//:apps //android/...
```

Without `--//:apps` nothing here is built, and without `ANDROID_HOME` in the
environment `bazel build //...` still passes: the server, the core's host
tests and the web client never touch the SDK.

After a change to the Maven artifacts in `MODULE.bazel`:

```sh
REPIN=1 bazel run --//:apps @maven_app//:pin
REPIN=1 bazel run --//:apps @maven_host//:pin
```

## Running on an emulator

```sh
# Once: an x86_64 system image and a device.
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" "system-images;android-35;google_apis;x86_64" emulator
"$ANDROID_HOME/cmdline-tools/latest/bin/avdmanager" create avd -n deappje -k "system-images;android-35;google_apis;x86_64"
"$ANDROID_HOME/emulator/emulator" -avd deappje &

bazel build --//:apps //android:app
"$ANDROID_HOME/platform-tools/adb" install -r bazel-bin/android/app.apk
"$ANDROID_HOME/platform-tools/adb" shell am start -n com.deappje.android/.ui.MainActivity
```

Sign in with Basable opens Kratos's flow in a Custom Tab; the browser
returns to `deappje://auth?code=...`, which the activity's intent filter
takes. The emulator's Keystore has Curve25519, so the device key is the
Keystore one there.

## Still by hand (later)

- **Store setup**: the Play Console app, the upload key and its keystore as
  GitHub secrets, an internal-track upload from `main`. The APK apps.yaml
  uploads is debug-signed with the rules' key.
- **Push**: a Firebase project, its `google-services.json`, the
  `firebase-messaging` dependency, and a `PushTokenSource` over it; the
  server side is `nanoservices/device` with its FCM secret.
- **An icon** and a launcher theme of the app's own.
- **Media, the calendar mirror, pairing by QR, export**: the shell's other
  duties in `docs/01-deappje/01-architecture.md`, "The shells".
