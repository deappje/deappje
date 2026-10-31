import type { StorageSeam } from "../../wasm/deappje_core.js";

/** The seam plus what the browser adds: waiting for the writes to land. */
export interface PersistentStorage extends StorageSeam {
  /** Resolves once every write so far is durable (or has failed). */
  flushed(): Promise<void>;
  /** Closes the database; the seam keeps answering from memory. */
  close(): void;
}

/**
 * Every blob in a Map, keys ordered: the core reads the whole store at
 * open and writes through on every change, so this is the one shape every
 * backend has, and the test's storage.
 */
export function memoryStorage(): StorageSeam & { readonly size: number } {
  const blobs = new Map<string, Uint8Array>();
  return {
    get size() {
      return blobs.size;
    },
    get: (key) => blobs.get(key),
    put: (key, value) => {
      blobs.set(key, value);
    },
    delete: (key) => {
      blobs.delete(key);
    },
    keys: (prefix) =>
      [...blobs.keys()]
        .filter((k) => k.startsWith(prefix))
        .sort(),
  };
}

const STORE = "blobs";

function request<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error ?? new Error("IndexedDB request failed"));
  });
}

function complete(tx: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error ?? new Error("IndexedDB transaction failed"));
    tx.onabort = () => reject(tx.error ?? new Error("IndexedDB transaction aborted"));
  });
}

/** Opens (or creates) a database with one object store, `store`. */
export function openDatabase(name: string, store: string, factory: IDBFactory = indexedDB): Promise<IDBDatabase> {
  const open = factory.open(name, 1);
  open.onupgradeneeded = () => {
    if (!open.result.objectStoreNames.contains(store)) open.result.createObjectStore(store);
  };
  return request(open);
}

/**
 * The store in IndexedDB. The seam is synchronous and IndexedDB is not, so
 * everything is read into memory here, before the core opens, and every
 * `put` and `delete` answers at once and lands in the database behind it,
 * in order. A crash between the two loses that one write; the core's next
 * sync repairs the rows and keeps the outbox entries that did land.
 *
 * OPFS is not used yet: on the main thread it is as asynchronous as this,
 * and its synchronous access handles exist only in a worker, which is
 * where this store moves when the core does.
 */
export async function openIndexedDbStorage(name = "deappje", factory: IDBFactory = indexedDB): Promise<PersistentStorage> {
  const db = await openDatabase(name, STORE, factory);
  const memory = memoryStorage();
  const tx = db.transaction(STORE, "readonly");
  const [keys, values] = await Promise.all([request(tx.objectStore(STORE).getAllKeys()), request(tx.objectStore(STORE).getAll())]);
  keys.forEach((key, i) => {
    const value = values[i];
    if (typeof key === "string" && value instanceof Uint8Array) memory.put(key, value);
  });
  // The writes, one after the other; a failed one is reported and the
  // chain goes on, so one bad write cannot stop every later one.
  let tail: Promise<void> = Promise.resolve();
  const enqueue = (write: (store: IDBObjectStore) => void) => {
    tail = tail.then(async () => {
      try {
        const tx = db.transaction(STORE, "readwrite");
        write(tx.objectStore(STORE));
        await complete(tx);
      } catch (e) {
        console.error("deappje: a write to IndexedDB failed", e);
      }
    });
  };
  return {
    get: (key) => memory.get(key),
    keys: (prefix) => memory.keys(prefix),
    put: (key, value) => {
      memory.put(key, value);
      enqueue((store) => store.put(value, key));
    },
    delete: (key) => {
      memory.delete(key);
      enqueue((store) => store.delete(key));
    },
    flushed: () => tail,
    close: () => db.close(),
  };
}
