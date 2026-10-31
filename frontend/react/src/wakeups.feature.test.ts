import { describe, it } from "vitest";

// Feature suite, "Wake-ups" (docs/01-deappje/03-testing.md): the shell's
// push dispatcher, a unit test, the phones only: the browser gets no push
// (core/AGENTS.md, "Wake-ups"), so this shell has no dispatcher and syncs
// on load, on focus and every 30 seconds instead (src/sync.ts).

describe("wake-ups", () => {
  it.skip("F50 reasons head, keys, rotate, history call sync; key_request refreshes the pending list; history_ready calls historyReady then sync; an unknown reason is ignored; a malformed payload is ignored and logged", () => {
    // The spec keeps F50 to the phones: the browser receives no push, so
    // useSync (load, focus, visibility, timer) is this shell's whole wake-up.
  });
});
