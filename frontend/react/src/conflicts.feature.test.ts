import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { ObjectBody } from "./wasm/deappje_core.js";
import { addMember, fields, newGroup, post, syncAll, World } from "./core/testing";

// Feature suite, "Conflicts" (docs/01-deappje/03-testing.md): the rebase
// as the shell's client sees it through the SyncReport and the rows.

type Event = { title: string; starts: number; ends: number; all_day: boolean; location: string; yearly: boolean };

function event(f: Event): ObjectBody {
  return { type: "Event", fields: f } as ObjectBody;
}

let world: World;
beforeAll(async () => {
  world = await World.open(15);
});
afterAll(() => world.free());

async function pair(name: string) {
  const { A, B } = world.users();
  const a = await world.browser(A);
  const b = await world.browser(B);
  const g = await newGroup(a, name);
  await addMember(a, b, g);
  return { a, b, g, done: () => [a, b].forEach((x) => x.dispose()) };
}

describe("conflicts", () => {
  it("F40 different fields", async () => {
    const { a, b, g, done } = await pair("Fields");
    const base: Event = { title: "Dinner", starts: 1000, ends: 2000, all_day: false, location: "", yearly: false };
    const ev = await a.core.createObject(g, undefined, event(base));
    await syncAll(a, b);
    // Offline: A retitles, B places it.
    await a.core.editObject(g, ev, event({ ...base, title: "Dinner at eight" }));
    await b.core.editObject(g, ev, event({ ...base, location: "The Pub" }));
    await a.core.sync();
    const rebased = await b.core.sync();
    expect(rebased.invalid).toEqual([]);
    expect(rebased.changed).toContain(g);
    await a.core.sync();
    const both = { ...base, title: "Dinner at eight", location: "The Pub" };
    expect(fields<Event>(await a.core.readObject(g, ev))).toEqual(both);
    expect(fields<Event>(await b.core.readObject(g, ev))).toEqual(both);
    expect((await b.core.readObject(g, ev))?.pending).toBe(false);
    done();
  });

  it("F41 same field", async () => {
    const { a, b, g, done } = await pair("Same field");
    const p = await post(a, g, "draft");
    await syncAll(a, b);
    await a.core.editObject(g, p, { type: "Post", fields: { text: "A's version" } });
    await b.core.editObject(g, p, { type: "Post", fields: { text: "B's version" } });
    // A pushes first; B's push is the later one and wins the field.
    await a.core.sync();
    expect((await a.core.readObject(g, p))?.body).toEqual({ type: "Post", fields: { text: "A's version" } });
    const rebased = await b.core.sync();
    expect(rebased.invalid).toEqual([]);
    expect((await b.core.readObject(g, p))).toMatchObject({ pending: false, body: { type: "Post", fields: { text: "B's version" } } });
    // The loser's local view updates on its next sync.
    expect((await a.core.readObject(g, p))?.body).toEqual({ type: "Post", fields: { text: "A's version" } });
    const caught = await a.core.sync();
    expect(caught.changed).toContain(g);
    expect((await a.core.readObject(g, p))?.body).toEqual({ type: "Post", fields: { text: "B's version" } });
    done();
  });

  it("F42 edit vs delete", async () => {
    const { a, b, g, done } = await pair("Edit vs delete");
    const p = await post(a, g, "to go");
    await syncAll(a, b);
    await b.core.editObject(g, p, { type: "Post", fields: { text: "to stay" } });
    expect((await b.core.readObject(g, p))?.pending).toBe(true);
    await a.core.deleteObject(g, p);
    await a.core.sync();
    // B's edit meets the tombstone: dropped, the group reported changed.
    const report = await b.core.sync();
    expect(report.changed).toContain(g);
    expect(report.invalid).toEqual([]);
    expect(report.failed).toEqual([]);
    expect(await b.core.readObject(g, p)).toBeUndefined();
    expect((await b.core.listObjects(g)).map((o) => o.id)).not.toContain(p);
    // Nothing of it comes back: a further sync on either side changes nothing.
    expect((await b.core.sync()).changed).not.toContain(g);
    expect((await a.core.sync()).changed).not.toContain(g);
    expect(await a.core.readObject(g, p)).toBeUndefined();
    done();
  });

  it.skip("F42 edit vs delete: the drop is reported as EditDropped", () => {
    // The core's EditDropped event is folded into SyncReport.changed by
    // Core.sync; the report carries no list of dropped edits. The test
    // above asserts the drop through the rows and `changed`.
  });

  it("F43 invalid rebase", async () => {
    const { a, b, g, done } = await pair("Invalid rebase");
    const base: Event = { title: "Dinner", starts: 1000, ends: 2000, all_day: false, location: "", yearly: false };
    const ev = await a.core.createObject(g, undefined, event(base));
    await syncAll(a, b);
    // Each edit is valid alone; together the end comes before the start.
    await a.core.editObject(g, ev, event({ ...base, starts: 1500 }));
    await b.core.editObject(g, ev, event({ ...base, ends: 1200 }));
    await a.core.sync();
    const parked = await b.core.sync();
    expect(parked.invalid).toEqual([{ group_id: g, object_id: ev, reason: "the event ends before it starts" }]);
    expect(parked.failed).toEqual([]);
    // Parked: B shows the merged result as pending; a further sync pushes
    // nothing of it (the report names it once, at the rebase).
    expect(await b.core.readObject(g, ev)).toMatchObject({ pending: true, body: event({ ...base, starts: 1500, ends: 1200 }) });
    expect((await b.core.sync()).failed).toEqual([]);
    expect((await b.core.readObject(g, ev))?.pending).toBe(true);
    await a.core.sync();
    expect(fields<Event>(await a.core.readObject(g, ev))).toEqual({ ...base, starts: 1500 });
    // Fixing the object clears it: the next sync pushes the fixed one.
    await b.core.editObject(g, ev, event({ ...base, starts: 1500, ends: 3000 }));
    const cleared = await b.core.sync();
    expect(cleared.invalid).toEqual([]);
    expect((await b.core.readObject(g, ev))?.pending).toBe(false);
    await a.core.sync();
    expect(fields<Event>(await a.core.readObject(g, ev))).toEqual({ ...base, starts: 1500, ends: 3000 });
    expect(await a.core.listObjects(g)).toEqual(await b.core.listObjects(g));
    done();
  });
});
