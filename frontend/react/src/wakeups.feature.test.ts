import { describe, it } from "vitest";

// Feature suite, "Wake-ups" (docs/01-deappje/03-testing.md): the shell's
// push dispatcher, a unit test. The browser gets no push (core/AGENTS.md,
// "Wake-ups"): this shell has no dispatcher, and syncs on load, on focus
// and every 30 seconds instead (src/sync.ts).

describe("wake-ups", () => {
  it.skip("F50 reasons head, keys, rotate, history call sync; key_request refreshes the pending list; history_ready calls historyReady then sync; an unknown reason is ignored; a malformed payload is ignored and logged", () => {
    // The web shell has no push dispatcher to test: the browser receives no
    // push, so useSync (load, focus, visibility, timer) is its whole wake-up.
  });
});
