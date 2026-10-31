import type { RandomSeam } from "../../wasm/deappje_core.js";

/** The platform's entropy: 32 bytes from `crypto.getRandomValues` per call. */
export const webRandom: RandomSeam = {
  randomBytes: () => crypto.getRandomValues(new Uint8Array(32)),
};
