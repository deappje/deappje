import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { type Browser, failure, newGroup, post, USER_A, World } from "./core/testing";

// Feature suite, "Session and device" (docs/01-deappje/03-testing.md): the
// shell's client over the wasm ModelServer, every seam in memory.

let world: World;
beforeAll(async () => {
  world = await World.open(11);
});
afterAll(() => world.free());

describe("session and device", () => {
  it("F01 sign in", async () => {
    const a = await world.browser(USER_A);
    const identity = await a.core.restoreSession();
    expect(identity).toMatchObject({ id: USER_A });
    expect(identity?.email).toBe(`${USER_A}@example`);
    expect(await a.core.userId()).toBe(USER_A);
    // Without a token there is no session: nobody.
    const nobody = world.anonymous();
    expect(await nobody.core.restoreSession()).toBeUndefined();
    expect(await nobody.core.userId()).toBeUndefined();
    a.dispose();
    nobody.dispose();
  });

  it("F02 register", async () => {
    const a = await world.browser(USER_A);
    const deviceId = await a.core.deviceId();
    const fingerprint = await a.core.fingerprint();
    expect(deviceId).toMatch(/^[0-9a-f-]{36}$/);
    expect(fingerprint).toMatch(/^[a-z0-9]{4}-[a-z0-9]{4}-[a-z0-9]{4}$/);
    // The harness registered once; twice more is still one device.
    await a.core.registerDevice("web");
    await a.core.registerDevice("web");
    expect(await a.core.deviceId()).toBe(deviceId);
    expect(await a.core.fingerprint()).toBe(fingerprint);
    expect(a.secrets.getSecret("device_id")).toBe(deviceId);
    // One device on the server: a second browser of the same user sees one
    // request from it, under that id and fingerprint, when it asks for keys.
    const a2 = await world.browser(USER_A);
    const g = await newGroup(a2, "Mine");
    await a.core.sync();
    const pending = (await a2.core.pendingKeyRequests()).filter((r) => r.group_id === g);
    expect(pending.map((r) => [r.device_id, r.fingerprint, r.platform])).toEqual([[deviceId, fingerprint, "web"]]);
    a.dispose();
    a2.dispose();
  });

  it("F03 reopen", async () => {
    const a = await world.browser(USER_A);
    const g = await newGroup(a, "Kept");
    await post(a, g, "before the reopen");
    await a.core.sync();
    const deviceId = await a.core.deviceId();
    const fingerprint = await a.core.fingerprint();
    const before = await a.core.listObjects(g);
    expect(before).toHaveLength(2);
    const keys = a.storage.keys(`k/${g}/`);
    expect(keys).toEqual([`k/${g}/1`]);
    a.dispose();

    // The same storage, secrets and key: the same device, signed in still.
    const again = await world.browser(USER_A, a);
    expect(await again.core.deviceId()).toBe(deviceId);
    expect(await again.core.fingerprint()).toBe(fingerprint);
    expect(again.storage.keys(`k/${g}/`)).toEqual(keys);
    expect(await again.core.listObjects(g)).toEqual(before);
    expect((await again.core.listGroups()).find((x) => x.id === g)).toEqual({ id: g, holds_keys: true, phase: "holder" });
    // And it goes on as that device: a sync changes nothing, a post lands.
    const report = await again.core.sync();
    expect(report).toMatchObject({ keys_received: [], started_over: [], failed: [] });
    await post(again, g, "after the reopen");
    await again.core.sync();
    expect(await again.core.listObjects(g)).toHaveLength(3);
    again.dispose();
  });

  it("F04 logout", async () => {
    const a = await world.browser(USER_A);
    const g = await newGroup(a, "Gone");
    await a.core.logout();
    expect(await a.core.restoreSession()).toBeUndefined();
    expect(await a.core.userId()).toBeUndefined();
    expect(a.secrets.getSecret("session_token")).toBeUndefined();
    const e = await failure(post(a, g, "nobody's"));
    expect(e).toMatchObject({ name: "CoreError", kind: "not_signed_in" });
    expect(await a.core.listObjects(g)).toHaveLength(1);
    // The server no longer knows the browser either.
    const listing = await failure(a.core.listGroups());
    expect(listing).toMatchObject({ kind: "api", code: "permission_denied" });
    a.dispose();
  });

  it("F05 outage", async () => {
    const a = await world.browser(USER_A);
    const g = await newGroup(a, "Flaky");
    const p = await post(a, g, "written offline");
    const before = await a.core.listObjects(g);
    expect(before.find((o) => o.id === p)).toMatchObject({ pending: true, seq: 0 });

    world.server.setOutage(true);
    const e = await failure(a.core.sync());
    expect(e).toMatchObject({ name: "CoreError", kind: "api", code: "unavailable" });
    // The store is as it was; the outbox keeps the write.
    expect(await a.core.listObjects(g)).toEqual(before);
    expect((await a.core.readObject(g, p))?.pending).toBe(true);

    world.server.setOutage(false);
    const report = await a.core.sync();
    expect(report.changed).toEqual([g]);
    expect(report.failed).toEqual([]);
    const landed = await a.core.readObject(g, p);
    expect(landed).toMatchObject({ pending: false, body: { type: "Post", fields: { text: "written offline" } } });
    expect(landed!.seq).toBeGreaterThan(0);
    a.dispose();
  });
});
