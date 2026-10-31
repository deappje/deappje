import type { Core } from "../wasm/deappje_core.js";
import type { CoreModule } from "./load";
import { systemClock } from "./seams/clock";
import { type DeviceKeyKind, newDeviceSecret, rememberedDeviceSecret, softwareDeviceKey } from "./seams/deviceKey";
import { webRandom } from "./seams/random";
import { memorySecrets, storedSecrets } from "./seams/secrets";
import { memoryStorage, openIndexedDbStorage, type PersistentStorage } from "./seams/storage";
import { fetchTransport } from "./seams/transport";

/** The core with the seams it was opened over, as the app holds it. */
export interface Client {
  core: Core;
  /** Whether the key and the store outlive the tab. */
  remember: boolean;
  deviceKey: DeviceKeyKind;
  storage: "indexeddb" | "memory";
  /** Waits for every write so far to be durable. */
  flush(): Promise<void>;
  /** Frees the core and closes the database. */
  dispose(): void;
}

/**
 * Opens the core as this browser: against the deployment's origin, over
 * `fetch`, the platform's entropy and clock, and, when the browser is
 * remembered, a device key and a store that come back next time; otherwise
 * both live for the session only.
 */
export async function openClient(module: CoreModule, { remember, origin }: { remember: boolean; origin: string }): Promise<Client> {
  const transport = fetchTransport(origin);
  if (!remember) {
    const core = new module.Core(transport, memoryStorage(), memorySecrets(), softwareDeviceKey(module, newDeviceSecret()), webRandom, systemClock, origin);
    return { core, remember, deviceKey: "software-memory", storage: "memory", flush: async () => {}, dispose: () => core.free() };
  }
  const [storage, secret]: [PersistentStorage, Uint8Array] = await Promise.all([openIndexedDbStorage(), rememberedDeviceSecret()]);
  const core = new module.Core(transport, storage, storedSecrets(localStorage), softwareDeviceKey(module, secret), webRandom, systemClock, origin);
  return {
    core,
    remember,
    deviceKey: "software-indexeddb",
    storage: "indexeddb",
    flush: () => storage.flushed(),
    dispose: () => {
      core.free();
      storage.close();
    },
  };
}
