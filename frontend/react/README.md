# The web client

The browser is a full device of the user (`docs/01-deappje/01-architecture.md`,
"The shells"): the shared Rust core (`core/`) compiled to wasm with
wasm-bindgen, and a Vite + React app that renders it. The app holds no
logic the core does not: it never parses an object value, never holds a
group key and never calls the server itself. It provides the six seams the
core needs from the platform (entropy, the clock, secrets, the device key,
the local store, HTTP) and draws screens from what the core answers.

Built by Bazel with rules_js into `dist/`, which the workflow uploads as
the `static` artifact: the platform serves it at `/` from the environment's
public bucket, answering a path that names no file with `index.html` (so
the app's own routes and a pasted invite link load), and the HTTPRoute
sends `/api/` to the app and `/.ory/` to Kratos. No image, no pod.

| Target | What it does |
|---|---|
| `//frontend/react:build` | the production bundle (`dist/`, the `.wasm` content-hashed under `dist/assets/`), the `static` artifact's content |
| `//frontend/react:wasm` | the core's glue and `.wasm` from `//core:core_wasm`, as `src/wasm/` (generated, never committed) |
| `//frontend/react:typecheck` | `tsc` over `src/`, tests included: Vite strips types without checking them, so this test is what fails on a type error |
| `//frontend/react:test` | vitest under Node: the seams' unit tests and the end-to-end test of two browsers over the core's in-memory `ModelServer` |
| `//frontend/react:start` | the dev server (`ibazel run //frontend/react:start`), `/api` proxied to an app on localhost:8080 and `/.ory` to a Kratos on localhost:4433 |

```sh
bazel test //frontend/react/...        # typecheck + tests (builds the wasm)
bazel build //frontend/react:build     # dist/
ibazel run //frontend/react:start      # the dev server, rebuilding the wasm on a core change
```

## How it is put together

```
src/
  wasm/            generated: deappje_core.js (+ .d.ts), deappje_core_bg.wasm
  core/
    load.ts        instantiates the wasm once
    client.ts      opens a Core over the seams, remembered or for the session
    session.ts     how the cookie session reaches the core; "remember this browser"
    seams/         random, clock, secrets, deviceKey, storage, transport
    e2e.test.ts    two browsers over the wasm ModelServer
  App.tsx          opens the client, routes, syncs
  router.ts        in-app navigation (no reload: a session-only key would be lost)
  sync.ts          sync on load, on focus and every 30 s while visible
  screens/         SignIn, Groups, Timeline, Approvals, ThisBrowser, Join
  Auth.tsx         Kratos's browser-flow pages (/auth/login, ...)
```

The wasm exports (`core/src/wasm.rs`) are the `Core` class with every
method of the facade as a Promise, `SoftwareDeviceKey` and `ModelServer`.
Records cross as plain objects with the core's field names (`group_id`,
`authored_at_ms`), ids as text, byte fields as number arrays, and a failure
rejects with an `Error` whose `kind` names the core's error variant
(`CoreError` in `src/wasm/deappje_core.d.ts`).

### The seams

| Seam | In the browser | In a test |
|---|---|---|
| Random | `crypto.getRandomValues` | the same (Node has it) |
| Clock | `Date.now()` | a fixed clock |
| Secrets | `localStorage` when remembered, memory otherwise | memory |
| DeviceKey | the core's own `SoftwareDeviceKey` over 32 secret bytes: in memory for the session, in IndexedDB when remembered | the same, from a chosen secret |
| Storage | IndexedDB, read whole into memory before the core opens, written through behind every `put`/`delete` | memory; IndexedDB through `fake-indexeddb` |
| Transport | `fetch` with the cookie for the origin, none for a presigned bucket URL | the wasm `ModelServer`, which is a Transport |

**Sign-in** is Kratos's browser flow: "Sign in with Basable" goes to
`/.ory/self-service/login/browser` and comes back with a cookie. The core
keeps a session *token* and sends it as `x-session-token`; the cookie is
not readable from JavaScript, so the app hands the core the marker
`cookie` (`session.ts`), the core asks whoami and registers the device as
if signed in by token, and the transport drops that one header so Kratos
and the API read the cookie. "Remember this browser" is chosen on the
sign-in screen, before the device exists.

## What it does and does not do yet

Does: sign in, register this browser as a `web` device, list and create
groups (the profile is the first object), join by a pasted or opened invite
link, approve pending key requests (fingerprint, platform, whether the
invite vouches), a group's timeline of posts (list, compose, edit and
delete one's own), sync on load, on focus and every 30 seconds, and the
"This browser" page with the fingerprint large enough for a phone to read.

Does not yet: the QR for pairing (the fingerprint is shown, not encoded),
photos and files, albums, todos, notes (the core's notes work through the
bindings; there is no screen), expenses, the calendar, polls, messages,
subgroups, members management, key rotation after a removal, logging out
a session-only device from the server (the core has no `remove_device`),
`navigator.storage.persist()`, and a Web Worker for the core (which is what
would make OPFS's synchronous access handles usable and the storage seam
truly synchronous).

Two things the design asks for wait on the core's seams: a
**non-extractable WebCrypto X25519 key** needs `DeviceKey::dh_bytes` to be
asynchronous (`crypto.subtle.deriveBits` returns a Promise), and
**OPFS** on the main thread is as asynchronous as IndexedDB. Until then the
key is held in software and the store is write-behind, as described above;
the "This browser" page says so.

## Dependencies

`package.json` pins exact versions and `pnpm-lock.yaml` is the resolved
graph rules_js builds from. To add a package, add it to `package.json`: the
verify build (`.basable/verify.sh`) resolves it into the lockfile and commits
the refreshed lock. By hand: `bazel run -- @pnpm --dir "$PWD/frontend/react"
install --lockfile-only`. Nothing in `BUILD.bazel` changes, since the build
depends on `:node_modules` as a whole. A package with an install script
needs an explicit answer (`allowBuilds`) in a `pnpm-workspace.yaml` next to
the lockfile; rules_js names the package when it is missing.
