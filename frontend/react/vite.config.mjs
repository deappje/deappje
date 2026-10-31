import { join } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Built by Bazel (//frontend/react:build) into dist/, which the workflow
// uploads as the `static` artifact: the platform serves it at "/" from the
// environment's public bucket. The core is a wasm build of core/ with
// wasm-bindgen's glue, copied into src/wasm/ (//frontend/react:wasm); the
// app imports the .wasm as an asset URL, so Vite emits it content-hashed
// under dist/assets/ and the glue fetches it from there.
//
// In production /api/ goes to the app and /.ory/ to Kratos through the
// HTTPRoute; the dev server proxies both to an app and a Kratos running
// locally. Vitest reads the `test` block (//frontend/react:test): Node, the
// seams' unit tests and the end-to-end test over the wasm ModelServer.
export default defineConfig({
  plugins: [react()],
  // Bazel's sandbox is read-only but for the test's temporary directory.
  cacheDir: process.env.TEST_TMPDIR ? join(process.env.TEST_TMPDIR, "vite") : "node_modules/.vite",
  server: {
    proxy: {
      "/api": "http://localhost:8080",
      "/.ory": "http://localhost:4433",
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
    watch: false,
  },
});
