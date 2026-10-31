import type { ClockSeam } from "../../wasm/deappje_core.js";

/** The wall clock. */
export const systemClock: ClockSeam = {
  nowMs: () => Date.now(),
};
