import type { CoreModule } from "../load";
import { openDatabase } from "./storage";
import { webRandom } from "./random";

const SECRET_DB = "deappje-device";
const SECRET_STORE = "device";
const SECRET_KEY = "x25519-secret";

/**
 * The device key is the core's `SoftwareDeviceKey`, X25519 over 32 secret
 * bytes the shell keeps. A non-extractable WebCrypto key is what the
 * design asks for, but the core's `DeviceKey` seam is synchronous and
 * `crypto.subtle.deriveBits` only answers a Promise, so until that seam
 * can wait, the secret is held in software: in memory for the session, or
 * in IndexedDB when the browser is remembered.
 */
export type DeviceKeyKind = "software-memory" | "software-indexeddb";

/** A fresh 32-byte secret. */
export function newDeviceSecret(): Uint8Array {
  return webRandom.randomBytes();
}

/** The remembered secret, minted and kept on first use. */
export async function rememberedDeviceSecret(factory: IDBFactory = indexedDB): Promise<Uint8Array> {
  const db = await openDatabase(SECRET_DB, SECRET_STORE, factory);
  try {
    const read = db.transaction(SECRET_STORE, "readonly");
    const kept = await new Promise<unknown>((resolve, reject) => {
      const r = read.objectStore(SECRET_STORE).get(SECRET_KEY);
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    });
    if (kept instanceof Uint8Array && kept.length === 32) return kept;
    const secret = newDeviceSecret();
    const write = db.transaction(SECRET_STORE, "readwrite");
    write.objectStore(SECRET_STORE).put(secret, SECRET_KEY);
    await new Promise<void>((resolve, reject) => {
      write.oncomplete = () => resolve();
      write.onerror = () => reject(write.error);
    });
    return secret;
  } finally {
    db.close();
  }
}

/** Forgets the remembered secret: the browser is a new device next time. */
export async function forgetDeviceSecret(factory: IDBFactory = indexedDB): Promise<void> {
  const db = await openDatabase(SECRET_DB, SECRET_STORE, factory);
  try {
    const tx = db.transaction(SECRET_STORE, "readwrite");
    tx.objectStore(SECRET_STORE).delete(SECRET_KEY);
    await new Promise<void>((resolve, reject) => {
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
  } finally {
    db.close();
  }
}

/** The seam over a secret: the core's X25519, the private half in this object. */
export function softwareDeviceKey(core: CoreModule, secret: Uint8Array) {
  return new core.SoftwareDeviceKey(secret);
}

/**
 * Whether this browser could hold the device key as a non-extractable
 * WebCrypto X25519 key, for the day the core's seam can wait on one. Shown
 * on the "This browser" page; decides nothing yet.
 */
export async function hasWebCryptoX25519(subtle: SubtleCrypto | undefined = globalThis.crypto?.subtle): Promise<boolean> {
  if (!subtle) return false;
  try {
    await subtle.generateKey({ name: "X25519" }, false, ["deriveBits"]);
    return true;
  } catch {
    return false;
  }
}
