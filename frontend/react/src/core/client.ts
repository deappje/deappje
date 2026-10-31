import type { ClockSeam, Core, DeviceKeySeam, RandomSeam, SecretsSeam, StorageSeam, TransportSeam } from "../wasm/deappje_core.js";
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

/** The six seams the core is opened over, and the origin it talks to. */
export interface Seams {
  transport: TransportSeam;
  storage: StorageSeam;
  secrets: SecretsSeam;
  deviceKey: DeviceKeySeam;
  random: RandomSeam;
  clock: ClockSeam;
  origin: string;
}

/** The persistent store's handle, when the key and the store outlive the tab. */
export type Kept = Pick<PersistentStorage, "flushed" | "close">;

/**
 * The core over the given seams: how every client is made, the browser's
 * (`openClient`) and a test's over the wasm `ModelServer` alike. With
 * `kept`, the store is the persistent one it names and the client waits
 * for its writes on `flush`; without, everything lives for the session.
 */
export function clientOver(module: CoreModule, seams: Seams, kept?: Kept): Client {
  const core = new module.Core(seams.transport, seams.storage, seams.secrets, seams.deviceKey, seams.random, seams.clock, seams.origin);
  if (!kept) {
    return { core, remember: false, deviceKey: "software-memory", storage: "memory", flush: async () => {}, dispose: () => core.free() };
  }
  return {
    core,
    remember: true,
    deviceKey: "software-indexeddb",
    storage: "indexeddb",
    flush: () => kept.flushed(),
    dispose: () => {
      core.free();
      kept.close();
    },
  };
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
    return clientOver(module, { transport, storage: memoryStorage(), secrets: memorySecrets(), deviceKey: softwareDeviceKey(module, newDeviceSecret()), random: webRandom, clock: systemClock, origin });
  }
  const [storage, secret]: [PersistentStorage, Uint8Array] = await Promise.all([openIndexedDbStorage(), rememberedDeviceSecret()]);
  return clientOver(module, { transport, storage, secrets: storedSecrets(localStorage), deviceKey: softwareDeviceKey(module, secret), random: webRandom, clock: systemClock, origin }, storage);
}
