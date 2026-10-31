import { describe, expect, it } from "vitest";
import { memorySecrets, storedSecrets, type StringStore } from "./secrets";

function fakeStore(): StringStore & { readonly entries: Map<string, string> } {
  const entries = new Map<string, string>();
  return {
    entries,
    getItem: (k) => entries.get(k) ?? null,
    setItem: (k, v) => void entries.set(k, v),
    removeItem: (k) => void entries.delete(k),
  };
}

describe("secrets", () => {
  it("memory: set, get, delete", () => {
    const s = memorySecrets();
    expect(s.getSecret("device_id")).toBeUndefined();
    s.setSecret("device_id", "d1");
    expect(s.getSecret("device_id")).toBe("d1");
    s.setSecret("device_id", "d2");
    expect(s.getSecret("device_id")).toBe("d2");
    s.deleteSecret("device_id");
    expect(s.getSecret("device_id")).toBeUndefined();
  });

  it("stored: the same, under a prefix in the string store", () => {
    const store = fakeStore();
    const s = storedSecrets(store);
    expect(s.getSecret("session_token")).toBeUndefined();
    s.setSecret("session_token", "cookie");
    expect(store.entries.get("deappje.secret.session_token")).toBe("cookie");
    expect(s.getSecret("session_token")).toBe("cookie");
    s.deleteSecret("session_token");
    expect(s.getSecret("session_token")).toBeUndefined();
    expect(store.entries.size).toBe(0);
  });
});
