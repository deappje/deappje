import { readFile } from "node:fs/promises";
import { type CoreModule, loadCore } from "./load";

/** The core for a test under Node: the .wasm read from beside its glue. */
export function loadCoreForTests(): Promise<CoreModule> {
  return loadCore(readFile(new URL("../wasm/deappje_core_bg.wasm", import.meta.url)).then((bytes) => new Uint8Array(bytes)));
}

/** Bytes from hex. */
export function hex(s: string): Uint8Array {
  return Uint8Array.from(s.match(/../g)!.map((b) => parseInt(b, 16)));
}

/** Hex from bytes. */
export function toHex(bytes: Uint8Array): string {
  return [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
}
