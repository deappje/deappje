import type { SecretsSeam } from "../../wasm/deappje_core.js";

/** Secrets for the session only: a browser that is not remembered. */
export function memorySecrets(): SecretsSeam {
  const values = new Map<string, string>();
  return {
    getSecret: (name) => values.get(name),
    setSecret: (name, value) => {
      values.set(name, value);
    },
    deleteSecret: (name) => {
      values.delete(name);
    },
  };
}

/** What `localStorage` offers; a test passes something smaller. */
export type StringStore = Pick<Storage, "getItem" | "setItem" | "removeItem">;

/**
 * Secrets kept across sessions in a string store (`localStorage` when the
 * browser is remembered), each under `prefix`. The session token kept here
 * is only the cookie marker (`session.ts`), never a credential.
 */
export function storedSecrets(store: StringStore, prefix = "deappje.secret."): SecretsSeam {
  return {
    getSecret: (name) => store.getItem(prefix + name) ?? undefined,
    setSecret: (name, value) => store.setItem(prefix + name, value),
    deleteSecret: (name) => store.removeItem(prefix + name),
  };
}
