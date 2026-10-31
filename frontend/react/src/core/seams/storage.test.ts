import { IDBFactory } from "fake-indexeddb";
import { describe, expect, it } from "vitest";
import { memoryStorage, openIndexedDbStorage } from "./storage";

const bytes = (...b: number[]) => Uint8Array.from(b);

describe("memoryStorage", () => {
  it("round-trips blobs and lists keys by prefix in order", () => {
    const s = memoryStorage();
    s.put("k/b", bytes(2));
    s.put("k/a", bytes(1));
    s.put("p/x", bytes(3));
    expect(s.get("k/a")).toEqual(bytes(1));
    expect(s.get("missing")).toBeUndefined();
    expect(s.keys("k/")).toEqual(["k/a", "k/b"]);
    expect(s.keys("")).toEqual(["k/a", "k/b", "p/x"]);
    s.delete("k/a");
    expect(s.keys("k/")).toEqual(["k/b"]);
    expect(s.size).toBe(2);
  });
});

describe("openIndexedDbStorage", () => {
  it("answers from memory at once and comes back whole after a reopen", async () => {
    const factory = new IDBFactory();
    const first = await openIndexedDbStorage("test", factory);
    first.put("k/g1/1", bytes(1, 2, 3));
    first.put("k/g1/2", bytes(4));
    first.put("p/d", bytes(5));
    first.delete("k/g1/2");
    // Synchronous answers, before anything landed.
    expect(first.get("k/g1/1")).toEqual(bytes(1, 2, 3));
    expect(first.keys("k/")).toEqual(["k/g1/1"]);
    await first.flushed();
    first.close();

    const second = await openIndexedDbStorage("test", factory);
    expect(second.keys("")).toEqual(["k/g1/1", "p/d"]);
    expect(second.get("k/g1/1")).toEqual(bytes(1, 2, 3));
    expect(second.get("k/g1/2")).toBeUndefined();
    second.put("k/g1/1", bytes(9));
    await second.flushed();
    second.close();

    const third = await openIndexedDbStorage("test", factory);
    expect(third.get("k/g1/1")).toEqual(bytes(9));
    third.close();
  });

  it("keeps databases apart by name", async () => {
    const factory = new IDBFactory();
    const a = await openIndexedDbStorage("a", factory);
    a.put("x", bytes(1));
    await a.flushed();
    const b = await openIndexedDbStorage("b", factory);
    expect(b.keys("")).toEqual([]);
    a.close();
    b.close();
  });
});
