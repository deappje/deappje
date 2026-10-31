import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { addMember, addOwnDevice, approve, type Browser, HOUR_MS, newGroup, ORIGIN, post, syncAll, WINDOW_MS, World } from "./core/testing";

// Feature suite, "A second device, history and the window"
// (docs/01-deappje/03-testing.md). `advance` moves the relay's clock,
// `sweep` runs its eviction and request expiry (core/AGENTS.md).

let world: World;
beforeAll(async () => {
  world = await World.open(13);
});
afterAll(() => world.free());

async function phase(b: Browser, g: string): Promise<string | undefined> {
  return (await b.core.listGroups()).find((x) => x.id === g)?.phase;
}

async function texts(b: Browser, g: string): Promise<string[]> {
  return (await b.core.listObjects(g)).filter((o) => o.body.type === "Post").map((o) => (o.body.fields as { text: string }).text);
}

describe("a second device, history and the window", () => {
  it("F13 own device", async () => {
    const { A } = world.users();
    const a = await world.browser(A);
    const g = await newGroup(a, "Mine");
    await post(a, g, "one");
    await post(a, g, "two");
    await a.core.sync();

    // A2 signs in as A's user, registers, syncs: it asks for the keys.
    const a2 = await world.browser(A);
    const asked = await a2.core.sync();
    expect(asked.keys_received).toEqual([]);
    expect(asked.failed).toEqual([]);
    expect((await a2.core.listGroups()).find((x) => x.id === g)).toEqual({ id: g, holds_keys: false, phase: "new" });
    expect(await a2.core.listObjects(g)).toEqual([]);

    const pending = (await a.core.pendingKeyRequests()).filter((r) => r.group_id === g);
    expect(pending).toHaveLength(1);
    expect(pending[0]).toMatchObject({ user_id: A, platform: "web", device_id: await a2.core.deviceId(), fingerprint: await a2.core.fingerprint() });
    // No invite vouches for a device of one's own: the fingerprint decides.
    // The core answers "none" (the field is absent), not false.
    expect(pending[0].invite_verified).not.toBe(true);
    expect(pending[0].invite_verified).toBeUndefined();
    await a.core.approveKeyRequest(pending[0].request_id);

    const report = await a2.core.sync();
    expect(report.keys_received).toEqual([g]);
    expect(report.changed).toContain(g);
    expect(await phase(a2, g)).toBe("holder");
    expect(await a2.core.listObjects(g)).toEqual(await a.core.listObjects(g));
    expect(await texts(a2, g)).toEqual(await texts(a, g));
    a.dispose();
    a2.dispose();
  });

  it("F14 catch-up after eviction", async () => {
    const { A, B } = world.users();
    const a = await world.browser(A);
    const a2 = await world.browser(A);
    const g = await newGroup(a, "Old timers");
    await addOwnDevice(a, a2, g);
    const posts = await Promise.all(["first", "second", "third"].map((t) => post(a, g, t)));
    await syncAll(a, a2);
    const held = await a.core.listObjects(g);
    expect(held).toHaveLength(4);

    // Every device acked, the window passes and the sweep empties the server.
    world.windowPasses();

    // B joins: the key comes, and the window holds only the invite secret
    // A pushed after the sweep; the rest is a holder's to give.
    const b = await world.browser(B);
    await addMember(a, b, g);
    expect(await phase(b, g)).toBe("catching_up");
    expect((await b.core.listObjects(g)).map((o) => o.body.type)).toEqual(["InviteSecret"]);
    const waiting = await b.core.sync();
    expect(waiting.failed).toEqual([]);
    expect(await phase(b, g)).toBe("catching_up");

    // A syncs: it serves the history; B's next sync makes it a holder.
    const served = await a.core.sync();
    expect(served.failed).toEqual([]);
    const done = await b.core.sync();
    expect(done.changed).toContain(g);
    expect(done.started_over).toEqual([]);
    expect(done.failed).toEqual([]);
    expect(await phase(b, g)).toBe("holder");
    const seen = await b.core.listObjects(g);
    for (const p of posts) {
      expect(seen.find((o) => o.id === p)).toMatchObject({ author: A, pending: false });
    }
    expect(seen.filter((o) => o.body.type !== "InviteSecret")).toEqual(held);
    a.dispose();
    a2.dispose();
    b.dispose();
  });

  it("F15 no wake-up", async () => {
    // The browser gets no push: this shell never calls historyReady, and
    // the catch-up completes on the next sync all the same.
    const { A, B } = world.users();
    const a = await world.browser(A);
    const g = await newGroup(a, "Quiet");
    const posts = await Promise.all(["first", "second"].map((t) => post(a, g, t)));
    await a.core.sync();
    world.windowPasses();

    const b = await world.browser(B);
    await addMember(a, b, g);
    expect(await phase(b, g)).toBe("catching_up");
    await a.core.sync();
    // No historyReady(requestId, throughSeq, complete) call: B's next sync
    // reads its request itself and finishes.
    const done = await b.core.sync();
    expect(done.changed).toContain(g);
    expect(await phase(b, g)).toBe("holder");
    for (const p of posts) {
      expect(await b.core.readObject(g, p)).toMatchObject({ author: A, pending: false });
    }
    expect((await b.core.listObjects(g)).filter((o) => o.body.type !== "InviteSecret")).toEqual((await a.core.listObjects(g)).filter((o) => o.body.type !== "InviteSecret"));
    a.dispose();
    b.dispose();
  });

  it("F16 joiners together", async () => {
    const { A, B, C } = world.users();
    const a = await world.browser(A);
    const g = await newGroup(a, "Crowd");
    const posts = await Promise.all(["first", "second", "third"].map((t) => post(a, g, t)));
    await a.core.sync();
    world.windowPasses();

    // Two links, then B and C join within the grace of each other: both
    // get the key and both wait on a holder (A does not sync in between,
    // or its first restore would already be in the window for the second).
    const b = await world.browser(B);
    const c = await world.browser(C);
    const links = [await a.core.createInvite(g, "member", 0, 0), await a.core.createInvite(g, "member", 0, 0)];
    await a.core.sync();
    await b.core.join(ORIGIN + links[0]);
    await c.core.join(ORIGIN + links[1]);
    await approve(a, b);
    await approve(a, c);
    expect((await b.core.sync()).keys_received).toEqual([g]);
    expect((await c.core.sync()).keys_received).toEqual([g]);
    expect(await phase(b, g)).toBe("catching_up");
    expect(await phase(c, g)).toBe("catching_up");

    // One sync of A serves both.
    const served = await a.core.sync();
    expect(served.failed).toEqual([]);
    for (const joiner of [b, c]) {
      const done = await joiner.core.sync();
      expect(done.changed).toContain(g);
      expect(await phase(joiner, g)).toBe("holder");
      for (const p of posts) {
        expect(await joiner.core.readObject(g, p)).toMatchObject({ author: A, pending: false });
      }
    }
    expect(await b.core.listObjects(g)).toEqual(await c.core.listObjects(g));
    a.dispose();
    b.dispose();
    c.dispose();
  });

  it.skip("F16 joiners together: A's SyncReport shows one request served", () => {
    // SyncReport has no count of history requests served: the core's
    // HistoryServed event is not surfaced by Core.sync. The test above
    // asserts the outcome (both joiners hold everything after one sync of A).
  });

  it("F17 start over", async () => {
    const { A, B } = world.users();
    const a = await world.browser(A);
    const b = await world.browser(B);
    const g = await newGroup(a, "Away");
    await addMember(a, b, g);
    const p1 = await post(a, g, "old");
    await syncAll(a, b);

    // B edits offline and stays away; A writes on, a window passes, A
    // writes again, and the sweep drops B's row with the old rows.
    await b.core.editObject(g, p1, { type: "Post", fields: { text: "old, edited away" } });
    const p2 = await post(a, g, "meanwhile");
    await a.core.sync();
    world.server.advance(WINDOW_MS + HOUR_MS);
    const p3 = await post(a, g, "fresh");
    await a.core.sync();
    world.server.sweep();

    const report = await b.core.sync();
    expect(report.started_over).toEqual([g]);
    expect(report.failed).toEqual([]);
    expect(await phase(b, g)).toBe("catching_up");
    // The window holds only the fresh row; the edit waits in the outbox.
    expect((await b.core.listObjects(g)).map((o) => o.id)).toEqual([p3]);

    // A serves the history; B catches up and its edit lands, rebased onto
    // the row as it is now.
    await a.core.sync();
    const done = await b.core.sync();
    expect(done.changed).toContain(g);
    expect(done.started_over).toEqual([]);
    expect(await phase(b, g)).toBe("holder");
    expect(await b.core.readObject(g, p1)).toMatchObject({ pending: false, body: { type: "Post", fields: { text: "old, edited away" } } });
    expect(await b.core.readObject(g, p2)).toMatchObject({ body: { type: "Post", fields: { text: "meanwhile" } } });
    await a.core.sync();
    expect((await a.core.readObject(g, p1))?.body).toEqual({ type: "Post", fields: { text: "old, edited away" } });
    expect(await a.core.listObjects(g)).toEqual(await b.core.listObjects(g));
    a.dispose();
    b.dispose();
  });
});
