export type { CoreError, GroupView, Identity, ObjectBody, ObjectView, PendingRequestView, SyncReport } from "../wasm/deappje_core.js";
export { type Client, openClient } from "./client";
export { type CoreModule, loadCore } from "./load";
export { COOKIE_SESSION, isRemembered, setRemembered, SIGN_OUT_URL, signInUrl } from "./session";
export { hasWebCryptoX25519 } from "./seams/deviceKey";

/** The core's failure, if that is what was thrown. */
export function asCoreError(e: unknown): import("../wasm/deappje_core.js").CoreError | undefined {
  return e instanceof Error && "kind" in e ? (e as import("../wasm/deappje_core.js").CoreError) : undefined;
}

/** What to show for a failure. */
export function describeError(e: unknown): string {
  const core = asCoreError(e);
  if (core) return core.message;
  return e instanceof Error ? e.message : String(e);
}
