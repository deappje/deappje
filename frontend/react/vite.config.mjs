import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Built by Bazel (//frontend/react:build) into dist/, which the workflow
// uploads as the `static` artifact: the platform serves it at "/" from the
// environment's public bucket. In production /api/ goes to the app through
// the HTTPRoute; the dev server proxies it to an app running locally.
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": "http://localhost:8080",
    },
  },
});
