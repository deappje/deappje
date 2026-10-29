# Adding a crate dependency

1. Add it to `[workspace.dependencies]` in the root `Cargo.toml` with a
   version. That list is the ONLY place versions live.
2. Reference it from the crate that uses it: `name = { workspace = true }`
   in that crate's `Cargo.toml`.
3. Nothing else: BUILD files use `all_crate_deps()`, and both
   `.basable/verify.sh` and the CI export `CARGO_BAZEL_REPIN=workspace`, so
   every build repins `Cargo.lock` and `Cargo.Bazel.lock`; commit both.
   Locally, `.basable/verify.sh` does it, or on its own:
   `CARGO_BAZEL_REPIN=workspace bazel mod tidy`.

Rules: never `openssl` (rustls everywhere), never a crate that needs a
system library (the image is distroless), never a second async runtime, and
never `tokio::sync::Mutex` or `RefCell` in a nanoservice crate (the clippy
aspect refuses them; `std::sync::Mutex`, atomics or `DashMap`, never held
across an `.await`).

# Adding an npm package (the React frontend)

1. Add it to `frontend/react/package.json` with an exact version.
2. Nothing else: the verify build resolves it into
   `frontend/react/pnpm-lock.yaml` and commits the lock, and the build
   depends on `:node_modules` as a whole. See `frontend/react/README.md`.
