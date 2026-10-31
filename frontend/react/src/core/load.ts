import init, * as wasm from "../wasm/deappje_core.js";

/** The core's wasm module: the `Core`, `ModelServer` and `SoftwareDeviceKey` classes and the record types. */
export type CoreModule = typeof wasm;

let loaded: Promise<CoreModule> | undefined;

/**
 * Instantiates the core once. `source` is the .wasm: its URL in the app
 * (Vite's `?url` import of `deappje_core_bg.wasm`), its bytes in a test.
 */
export function loadCore(source: wasm.InitInput | Promise<wasm.InitInput>): Promise<CoreModule> {
  loaded ??= init({ module_or_path: source }).then(() => wasm);
  return loaded;
}
