import { createHash } from "node:crypto";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { FileRef, ObjectBody, ObjectTypeName } from "./wasm/deappje_core.js";
import { addMember, addOwnDevice, type Browser, failure, fields, invite, newGroup, ofType, post, syncAll, World } from "./core/testing";

// Feature suite, "Object types" (docs/01-deappje/03-testing.md): one
// scenario per type, each create, readObject, listObjects by type, edit
// where the strategy allows, delete, and the other device seeing each
// step. Field names are the core's (docs/01-deappje/06-object-types.md,
// core/src/types.rs); byte fields cross as number arrays, ids as text,
// enums as their variant names.

type Geo = { lat: number; lng: number };
type Photo = { width: number; height: number; taken_at?: number; caption: string; thumbnail: number[]; full?: FileRef };
type Album = { title: string; cover_photo_id?: string };
type TodoItem = { text: string; done: boolean; assignee?: string; due?: number; pos: number };
type Participant = { user_id: string; share: number };
type Expense = { amount: number; currency: string; paid_by: string; participants: Participant[]; split: "Equal" | "Weights" | "Exact"; kind: "Expense" | "Ticket"; description: string; date: number };
type Settlement = { from: string; to: string; amount: number; currency: string; date: number };
type TicketShare = { holder_user_id?: string; holder_name: string; status: "Held" | "Offered" | "Used"; file?: FileRef };
type Event = { title: string; starts: number; ends: number; all_day: boolean; location: string; geo?: Geo; yearly: boolean };
type PollOption = { id: string; text: string; slot?: number; pos: number };
type Poll = { question: string; kind: "Choice" | "Dates"; options: PollOption[]; multi: boolean; closes_at?: number };
type Vote = { answers: { option_id: string; answer: "No" | "Yes" | "Maybe" }[] };
type Message = { text: string; reply_to?: string };
type Venue = { name: string; category: "Other" | "Bar" | "Pub" | "Club" | "Restaurant"; address: string; geo?: Geo; url: string; notes: string };
type MemberProfile = { display_name: string; avatar: number[]; birthday?: { month: number; day: number; year?: number } };
type InviteSecret = { invite_id: string; secret: number[] };

const DAY = 24 * 60 * 60 * 1000;

function body(type: ObjectTypeName, fieldsOf: object): ObjectBody {
  return { type, fields: fieldsOf } as ObjectBody;
}

function uuidBytes(id: string): Buffer {
  return Buffer.from(id.replace(/-/g, ""), "hex");
}

/** RFC 4122 version 5: SHA-1 over the namespace's bytes and the name, with the version and variant bits set. */
function uuidV5(namespace: string, name: Uint8Array): string {
  const h = createHash("sha1").update(uuidBytes(namespace)).update(name).digest().subarray(0, 16);
  h[6] = (h[6] & 0x0f) | 0x50;
  h[8] = (h[8] & 0x3f) | 0x80;
  const x = h.toString("hex");
  return `${x.slice(0, 8)}-${x.slice(8, 12)}-${x.slice(12, 16)}-${x.slice(16, 20)}-${x.slice(20)}`;
}

let world: World;
beforeAll(async () => {
  world = await World.open(14);
});
afterAll(() => world.free());

/** A and B in a group B joined; every step below is seen from the other device after a sync. */
async function pair(name: string) {
  const { A, B, C } = world.users();
  const a = await world.browser(A);
  const b = await world.browser(B);
  const g = await newGroup(a, name);
  await addMember(a, b, g);
  return { a, b, g, A, B, C, done: () => [a, b].forEach((x) => x.dispose()) };
}

async function photo(by: Browser, g: string, parent: string, caption = ""): Promise<string> {
  return by.core.createObject(g, parent, body("Photo", { width: 640, height: 480, caption, thumbnail: [1, 2, 3] }));
}

describe("object types", () => {
  it("F20 post", async () => {
    const { a, b, g, A, done } = await pair("Posts");
    const p = await post(a, g, "hello");
    const mine = await a.core.readObject(g, p);
    expect(mine).toMatchObject({ id: p, author: A, pending: true, seq: 0, body: { type: "Post", fields: { text: "hello" } } });
    expect(mine!.parent_id).toBeUndefined();
    expect(mine!.edited_at_ms).toBe(mine!.authored_at_ms);
    await syncAll(a, b);
    expect(await b.core.readObject(g, p)).toMatchObject({ author: A, pending: false, body: { type: "Post", fields: { text: "hello" } } });
    expect((await ofType(b, g, "Post")).map((o) => o.id)).toEqual([p]);

    // The author's device edits; the other sees the edit and its time.
    await a.core.editObject(g, p, { type: "Post", fields: { text: "hello, edited" } });
    await syncAll(a, b);
    const edited = await b.core.readObject(g, p);
    expect(edited?.body).toEqual({ type: "Post", fields: { text: "hello, edited" } });
    expect(edited!.edited_at_ms).toBeGreaterThan(edited!.authored_at_ms);
    expect(edited!.authored_at_ms).toBe(mine!.authored_at_ms);

    // A comment and a photo under it, from B; A's delete takes them along.
    const c = await b.core.createObject(g, p, { type: "Comment", fields: { text: "nice" } });
    const ph = await photo(b, g, p);
    await syncAll(b, a);
    expect((await a.core.readObject(g, c))?.parent_id).toBe(p);
    expect((await a.core.readObject(g, ph))?.parent_id).toBe(p);
    await a.core.deleteObject(g, p);
    for (const id of [p, c, ph]) expect(await a.core.readObject(g, id)).toBeUndefined();
    await syncAll(a, b);
    for (const id of [p, c, ph]) expect(await b.core.readObject(g, id)).toBeUndefined();
    expect(await ofType(b, g, "Post")).toEqual([]);
    expect(await ofType(b, g, "Comment")).toEqual([]);
    done();
  });

  it.skip("F20 post: only the author's devices edit", () => {
    // The core's editObject accepts any member's edit of a post (the e2e
    // test has B editing A's post); the web shell keeps the rule only in
    // the Timeline screen, which offers Edit and Delete on one's own posts
    // (F63). Nothing in the client refuses another author's edit.
  });

  it("F21 comment", async () => {
    const { a, b, g, B, done } = await pair("Comments");
    const p = await post(a, g, "a post");
    const ph = await photo(a, g, p);
    const v = await a.core.createObject(g, undefined, body("Venue", { name: "The Pub", category: "Pub", address: "", url: "", notes: "" }));
    await syncAll(a, b);
    const c1 = await b.core.createObject(g, p, { type: "Comment", fields: { text: "on the post" } });
    const c2 = await b.core.createObject(g, ph, { type: "Comment", fields: { text: "on the photo" } });
    const c3 = await b.core.createObject(g, v, { type: "Comment", fields: { text: "on the venue" } });
    expect(await failure(b.core.createObject(g, undefined, { type: "Comment", fields: { text: "orphan" } }))).toMatchObject({ kind: "invalid" });
    expect(await failure(b.core.createObject(g, p, { type: "Comment", fields: { text: "  " } }))).toMatchObject({ kind: "invalid" });
    await syncAll(b, a);
    const seen = await ofType(a, g, "Comment");
    expect(new Map(seen.map((o) => [o.id, o.parent_id]))).toEqual(new Map([[c1, p], [c2, ph], [c3, v]]));
    expect(seen.every((o) => o.author === B)).toBe(true);

    await b.core.editObject(g, c1, { type: "Comment", fields: { text: "on the post, edited" } });
    await syncAll(b, a);
    expect((await a.core.readObject(g, c1))?.body).toEqual({ type: "Comment", fields: { text: "on the post, edited" } });

    // Each goes with its parent.
    await a.core.deleteObject(g, v);
    await a.core.deleteObject(g, ph);
    await syncAll(a, b);
    expect(await b.core.readObject(g, c3)).toBeUndefined();
    expect(await b.core.readObject(g, c2)).toBeUndefined();
    expect((await b.core.readObject(g, c1))?.parent_id).toBe(p);
    await b.core.deleteObject(g, c1);
    await syncAll(b, a);
    expect(await ofType(a, g, "Comment")).toEqual([]);
    expect(await a.core.readObject(g, p)).toBeDefined();
    done();
  });

  it("F22 reaction", async () => {
    const { a, b, g, A, B, done } = await pair("Reactions");
    const p = await post(a, g, "react to this");
    await syncAll(a, b);
    // The core derives the per-user singleton id: UUIDv5(parent, user).
    const r1 = await a.core.createObject(g, p, body("Reaction", { emoji: "👍" }));
    expect(r1).toBe(uuidV5(p, uuidBytes(A)));
    await syncAll(a, b);
    expect(await b.core.readObject(g, r1)).toMatchObject({ parent_id: p, author: A, body: { type: "Reaction", fields: { emoji: "👍" } } });

    // A second reaction by the same user is an edit of the same object:
    // the shell finds its own under the parent and edits it.
    const mine = (await ofType(a, g, "Reaction")).find((o) => o.parent_id === p && o.author === A);
    expect(mine?.id).toBe(r1);
    await a.core.editObject(g, r1, body("Reaction", { emoji: "❤️" }));
    await syncAll(a, b);
    expect((await ofType(b, g, "Reaction")).filter((o) => o.parent_id === p && o.author === A)).toHaveLength(1);
    expect(fields<{ emoji: string }>(await b.core.readObject(g, r1)).emoji).toBe("❤️");

    // Two users' reactions are two objects; A's other device reads both.
    const r2 = await b.core.createObject(g, p, body("Reaction", { emoji: "😂" }));
    expect(r2).toBe(uuidV5(p, uuidBytes(B)));
    expect(r2).not.toBe(r1);
    await syncAll(b, a);
    expect((await ofType(a, g, "Reaction")).map((o) => o.id).sort()).toEqual([r1, r2].sort());
    const a2 = await world.browser(A);
    await addOwnDevice(a, a2, g);
    expect((await ofType(a2, g, "Reaction")).map((o) => [o.id, o.author]).sort()).toEqual([[r1, A], [r2, B]].sort());
    expect(await failure(a.core.createObject(g, p, body("Reaction", { emoji: "" })))).toMatchObject({ kind: "invalid" });

    await b.core.deleteObject(g, r2);
    await syncAll(b, a);
    expect(await a.core.readObject(g, r2)).toBeUndefined();
    expect(await a.core.readObject(g, r1)).toBeDefined();
    a2.dispose();
    done();
  });

  it("F23 photo", async () => {
    const { a, b, g, B, done } = await pair("Photos");
    const p = await post(a, g, "with a photo");
    await syncAll(a, b);
    const thumbnail = Array.from({ length: 64 }, (_, i) => (i * 7) % 256);
    const ph = await b.core.createObject(g, p, body("Photo", { width: 640, height: 480, caption: "us", thumbnail }));
    expect(await failure(b.core.createObject(g, p, body("Photo", { width: 0, height: 480, caption: "", thumbnail: [] })))).toMatchObject({ kind: "invalid" });
    const bytes = Uint8Array.from({ length: 1000 }, (_, i) => i % 251);
    const file = await b.core.uploadFile(g, ph, "full", bytes);
    expect(file).toMatchObject({ slot: "full", size: 1000 + 24 + 16 });
    expect(file.sha256).toHaveLength(32);
    expect(file.content_key).toHaveLength(32);
    expect(world.server.hasBlob(`g/${g}/o/${ph}/full`)).toBe(true);
    await b.core.editObject(g, ph, body("Photo", { width: 640, height: 480, caption: "us", thumbnail, full: file }));
    await syncAll(b, a);

    const seen = await a.core.readObject(g, ph);
    expect(seen).toMatchObject({ parent_id: p, author: B, pending: false });
    const f = fields<Photo>(seen);
    expect(f.thumbnail).toEqual(thumbnail);
    expect(f.full).toEqual(file);
    expect([f.width, f.height, f.caption]).toEqual([640, 480, "us"]);
    expect(await a.core.downloadFile(g, ph, f.full!)).toEqual(bytes);
    expect((await ofType(a, g, "Photo")).map((o) => o.id)).toEqual([ph]);
    // The caption is the photo's editable part.
    await a.core.editObject(g, ph, body("Photo", { ...f, caption: "all of us" }));
    await syncAll(a, b);
    expect(fields<Photo>(await b.core.readObject(g, ph)).caption).toBe("all of us");
    expect(fields<Photo>(await b.core.readObject(g, ph)).full).toEqual(file);

    await b.core.deleteObject(g, ph);
    await syncAll(b, a);
    expect(await a.core.readObject(g, ph)).toBeUndefined();
    expect(await ofType(a, g, "Photo")).toEqual([]);
    done();
  });

  it.skip("F23 photo: the file is gone from the bucket after the photo's delete", () => {
    // Neither the core nor the ModelServer removes a blob on a tombstone:
    // hasBlob stays true after the delete. 02-relay.md gives the cleanup to
    // the storage service, which the model does not have.
  });

  it("F24 album", async () => {
    const { a, b, g, done } = await pair("Albums");
    const al = await a.core.createObject(g, undefined, body("Album", { title: "Summer" }));
    expect(await failure(a.core.createObject(g, undefined, body("Album", { title: " " })))).toMatchObject({ kind: "invalid" });
    const ph1 = await photo(a, g, al, "beach");
    const ph2 = await photo(a, g, al, "hills");
    await syncAll(a, b);
    expect(fields<Album>(await b.core.readObject(g, al))).toEqual({ title: "Summer" });
    expect((await ofType(b, g, "Photo")).map((o) => o.parent_id)).toEqual([al, al]);
    expect((await ofType(b, g, "Album")).map((o) => o.id)).toEqual([al]);

    // Offline: A renames, B picks a cover; the rebase keeps both.
    await a.core.editObject(g, al, body("Album", { title: "Summer 2026" }));
    await b.core.editObject(g, al, body("Album", { title: "Summer", cover_photo_id: ph1 }));
    await a.core.sync();
    const rebased = await b.core.sync();
    expect(rebased.invalid).toEqual([]);
    await a.core.sync();
    expect(fields<Album>(await a.core.readObject(g, al))).toEqual({ title: "Summer 2026", cover_photo_id: ph1 });
    expect(fields<Album>(await b.core.readObject(g, al))).toEqual({ title: "Summer 2026", cover_photo_id: ph1 });

    await a.core.deleteObject(g, al);
    await syncAll(a, b);
    for (const id of [al, ph1, ph2]) expect(await b.core.readObject(g, id)).toBeUndefined();
    done();
  });

  it("F25 todo list and items", async () => {
    const { a, b, g, B, done } = await pair("Todos");
    const list = await a.core.createObject(g, undefined, body("TodoList", { title: "Shopping" }));
    const item = (text: string, pos: number) => a.core.createObject(g, list, body("TodoItem", { text, done: false, pos }));
    const i1 = await item("milk", 1);
    const i2 = await item("eggs", 2);
    const i3 = await item("bread", 3);
    expect(await failure(a.core.createObject(g, undefined, body("TodoItem", { text: "orphan", done: false, pos: 1 })))).toMatchObject({ kind: "invalid" });
    await syncAll(a, b);
    const ordered = async (by: Browser) =>
      (await ofType(by, g, "TodoItem"))
        .filter((o) => o.parent_id === list)
        .sort((x, y) => (x.body.fields as TodoItem).pos - (y.body.fields as TodoItem).pos)
        .map((o) => (o.body.fields as TodoItem).text);
    expect(await ordered(b)).toEqual(["milk", "eggs", "bread"]);

    // Tick and untick the same item offline. A boolean changes from one
    // base in one direction only, so the ticks agree; the later push
    // settles the field, and B's text edit is kept beside A's tick.
    const current = async (by: Browser, id: string) => fields<TodoItem>(await by.core.readObject(g, id));
    await a.core.editObject(g, i1, body("TodoItem", { ...(await current(a, i1)), done: true }));
    await b.core.editObject(g, i1, body("TodoItem", { ...(await current(b, i1)), done: true, assignee: B }));
    await a.core.sync();
    expect((await b.core.sync()).invalid).toEqual([]);
    await a.core.sync();
    expect(await current(a, i1)).toMatchObject({ done: true, assignee: B });
    expect(await current(b, i1)).toMatchObject({ done: true, assignee: B });
    // The untick, from the ticked base, lands on the other device.
    await b.core.editObject(g, i1, body("TodoItem", { ...(await current(b, i1)), done: false }));
    await syncAll(b, a);
    expect((await current(a, i1)).done).toBe(false);

    // A move is a `pos` edit: both devices move "bread" offline, the
    // later push wins, and the list has three items either way.
    await a.core.editObject(g, i3, body("TodoItem", { ...(await current(a, i3)), pos: 0.5 }));
    await b.core.editObject(g, i3, body("TodoItem", { ...(await current(b, i3)), pos: 1.5 }));
    await a.core.sync();
    await b.core.sync();
    await a.core.sync();
    expect(await ordered(a)).toEqual(["milk", "bread", "eggs"]);
    expect(await ordered(b)).toEqual(["milk", "bread", "eggs"]);
    expect((await current(a, i3)).pos).toBe(1.5);
    expect(await ofType(a, g, "TodoItem")).toHaveLength(3);

    await a.core.deleteObject(g, i2);
    await syncAll(a, b);
    expect(await ordered(b)).toEqual(["milk", "bread"]);
    await b.core.deleteObject(g, list);
    await syncAll(b, a);
    expect(await ofType(a, g, "TodoItem")).toEqual([]);
    expect(await a.core.readObject(g, list)).toBeUndefined();
    done();
  });

  it("F26 note", async () => {
    const { a, b, g, done } = await pair("Notes");
    const n = await a.core.createObject(g, undefined, { type: "Note", fields: { title: "Plan" } });
    expect(await a.core.saveNote(g, n, "milk\n")).toBe(true);
    expect(await a.core.saveNote(g, n, "milk\n")).toBe(false);
    await syncAll(a, b);
    expect(await b.core.readNote(g, n)).toBe("milk\n");
    expect((await ofType(b, g, "NoteUpdate")).map((o) => o.parent_id)).toEqual([n]);

    // Offline, different insertions on each device: one text afterwards.
    expect(await a.core.saveNote(g, n, "milk\neggs\n")).toBe(true);
    expect(await b.core.saveNote(g, n, "bread\nmilk\n")).toBe(true);
    await syncAll(a, b);
    await syncAll(a, b);
    const text = await a.core.readNote(g, n);
    expect(text).toBe("bread\nmilk\neggs\n");
    expect(await b.core.readNote(g, n)).toBe(text);
    expect(await ofType(a, g, "NoteUpdate")).toHaveLength(3);

    // A title edit is a rebase beside B's update.
    await a.core.editObject(g, n, { type: "Note", fields: { title: "Plan B" } });
    await b.core.saveNote(g, n, "bread\nmilk\neggs\nbutter\n");
    await syncAll(a, b);
    await syncAll(a, b);
    expect((await b.core.readObject(g, n))?.body).toEqual({ type: "Note", fields: { title: "Plan B" } });
    expect(await a.core.readNote(g, n)).toBe("bread\nmilk\neggs\nbutter\n");

    // The note's delete cascades to its updates.
    await a.core.deleteObject(g, n);
    await syncAll(a, b);
    expect(await b.core.readObject(g, n)).toBeUndefined();
    expect(await ofType(b, g, "NoteUpdate")).toEqual([]);
    expect(await ofType(a, g, "NoteUpdate")).toEqual([]);
    done();
  });

  it("F27 expense pool, expense, settlement", async () => {
    const { a, b, g, A, B, C, done } = await pair("Expenses");
    const poolId = a.core.generalPoolId(g);
    expect(poolId).toBe(b.core.generalPoolId(g));
    expect(poolId).toBe(uuidV5(g, Buffer.from("pool")));
    await a.core.createObjectWithId(g, poolId, undefined, body("ExpensePool", { title: "General", archived: false }));
    expect(await a.core.readObject(g, poolId)).toMatchObject({ id: poolId, body: { type: "ExpensePool", fields: { title: "General", archived: false } } });

    const date = 1_800_000 * 1000;
    const dinner: Expense = { amount: 3000, currency: "EUR", paid_by: A, participants: [{ user_id: A, share: 0 }, { user_id: B, share: 0 }], split: "Equal", kind: "Expense", description: "dinner", date };
    const e1 = await a.core.createObject(g, poolId, body("Expense", dinner));
    const e2 = await a.core.createObject(g, poolId, body("Expense", { ...dinner, description: "taxi", split: "Weights", participants: [{ user_id: A, share: 1 }, { user_id: B, share: 2 }] }));
    const e3 = await a.core.createObject(g, poolId, body("Expense", { ...dinner, description: "tickets", split: "Exact", participants: [{ user_id: A, share: 1000 }, { user_id: B, share: 2000 }] }));
    expect(await failure(a.core.createObject(g, poolId, body("Expense", { ...dinner, split: "Exact", participants: [{ user_id: A, share: 1000 }, { user_id: B, share: 1000 }] })))).toMatchObject({ kind: "invalid" });
    expect(await failure(a.core.createObject(g, poolId, body("Expense", { ...dinner, currency: "eur" })))).toMatchObject({ kind: "invalid" });
    expect(await failure(a.core.createObject(g, undefined, body("Expense", dinner)))).toMatchObject({ kind: "invalid" });
    await syncAll(a, b);
    expect((await ofType(b, g, "Expense")).map((o) => o.parent_id)).toEqual([poolId, poolId, poolId]);
    expect(fields<Expense>(await b.core.readObject(g, e1))).toEqual(dinner);
    expect(fields<Expense>(await b.core.readObject(g, e2)).split).toBe("Weights");
    expect(fields<Expense>(await b.core.readObject(g, e3)).participants).toEqual([{ user_id: A, share: 1000 }, { user_id: B, share: 2000 }]);

    // Offline, A changes the amount and B adds a participant; both kept.
    await a.core.editObject(g, e1, body("Expense", { ...dinner, amount: 3600 }));
    await b.core.editObject(g, e1, body("Expense", { ...dinner, participants: [...dinner.participants, { user_id: C, share: 0 }] }));
    await a.core.sync();
    expect((await b.core.sync()).invalid).toEqual([]);
    await a.core.sync();
    const merged = { ...dinner, amount: 3600, participants: [{ user_id: A, share: 0 }, { user_id: B, share: 0 }, { user_id: C, share: 0 }] };
    expect(fields<Expense>(await a.core.readObject(g, e1))).toEqual(merged);
    expect(fields<Expense>(await b.core.readObject(g, e1))).toEqual(merged);

    // A settlement is append-only: deleted, never edited.
    const s: Settlement = { from: B, to: A, amount: 1500, currency: "EUR", date };
    const st = await b.core.createObject(g, poolId, body("Settlement", s));
    expect(await failure(b.core.createObject(g, poolId, body("Settlement", { ...s, to: B })))).toMatchObject({ kind: "invalid" });
    await syncAll(b, a);
    expect(await a.core.readObject(g, st)).toMatchObject({ parent_id: poolId, author: B, body: { type: "Settlement", fields: s } });
    await a.core.deleteObject(g, st);
    await syncAll(a, b);
    expect(await b.core.readObject(g, st)).toBeUndefined();

    // A ticket kind expense splits equally, or is refused.
    const tickets = await a.core.createObject(g, poolId, body("Expense", { ...dinner, description: "concert", kind: "Ticket" }));
    expect(await failure(a.core.createObject(g, poolId, body("Expense", { ...dinner, kind: "Ticket", split: "Weights", participants: [{ user_id: A, share: 1 }, { user_id: B, share: 2 }] })))).toMatchObject({ kind: "invalid" });
    await syncAll(a, b);
    expect(fields<Expense>(await b.core.readObject(g, tickets)).kind).toBe("Ticket");
    await a.core.deleteObject(g, e2);
    await syncAll(a, b);
    expect((await ofType(b, g, "Expense")).map((o) => o.id).sort()).toEqual([e1, e3, tickets].sort());
    done();
  });

  it.skip("F27 settlement: an edit is refused", () => {
    // The core's editObject does not refuse an append-only type: a
    // settlement (and an invite secret, a note update) can be edited through
    // it. The strategy lives in the core (Strategy::AppendOnly); the shell
    // has no edit path for a settlement to refuse in.
  });

  it("F28 ticket share", async () => {
    const { a, b, g, A, B, done } = await pair("Tickets");
    const poolId = a.core.generalPoolId(g);
    await a.core.createObjectWithId(g, poolId, undefined, body("ExpensePool", { title: "General", archived: false }));
    const concert = await a.core.createObject(g, poolId, body("Expense", { amount: 9000, currency: "EUR", paid_by: A, participants: [{ user_id: A, share: 0 }, { user_id: B, share: 0 }], split: "Equal", kind: "Ticket", description: "concert", date: 0 }));
    const share = await a.core.createObject(g, concert, body("TicketShare", { holder_user_id: A, holder_name: "", status: "Held" }));
    expect(await failure(a.core.createObject(g, concert, body("TicketShare", { holder_name: "", status: "Held" })))).toMatchObject({ kind: "invalid" });
    const pdf = Uint8Array.from({ length: 300 }, (_, i) => (i * 13) % 256);
    const file = await a.core.uploadFile(g, share, "file", pdf);
    expect(world.server.hasBlob(`g/${g}/o/${share}/file`)).toBe(true);
    await a.core.editObject(g, share, body("TicketShare", { holder_user_id: A, holder_name: "", status: "Held", file }));
    await syncAll(a, b);
    const seen = fields<TicketShare>(await b.core.readObject(g, share));
    expect(seen).toMatchObject({ holder_user_id: A, status: "Held" });
    expect(seen.file).toEqual(file);
    expect(await b.core.downloadFile(g, share, seen.file!)).toEqual(pdf);
    expect((await ofType(b, g, "TicketShare")).map((o) => o.parent_id)).toEqual([concert]);

    // A transfer edits the holder; the new holder marks it used.
    await a.core.editObject(g, share, body("TicketShare", { ...seen, holder_user_id: B, status: "Offered" }));
    await syncAll(a, b);
    expect(fields<TicketShare>(await b.core.readObject(g, share))).toMatchObject({ holder_user_id: B, status: "Offered", file });
    await b.core.editObject(g, share, body("TicketShare", { ...seen, holder_user_id: B, status: "Used" }));
    await syncAll(b, a);
    expect(fields<TicketShare>(await a.core.readObject(g, share)).status).toBe("Used");
    // A plus-one holds by name.
    const cousin = await a.core.createObject(g, concert, body("TicketShare", { holder_name: "Bob's cousin", status: "Held" }));
    await syncAll(a, b);
    expect(fields<TicketShare>(await b.core.readObject(g, cousin))).toMatchObject({ holder_name: "Bob's cousin" });

    await a.core.deleteObject(g, concert);
    await syncAll(a, b);
    expect(await ofType(b, g, "TicketShare")).toEqual([]);
    done();
  });

  it("F29 event", async () => {
    const { a, b, g, done } = await pair("Calendar");
    const dinner: Event = { title: "Dinner", starts: 10 * DAY, ends: 10 * DAY + 2 * 3600 * 1000, all_day: false, location: "The Pub", yearly: false };
    const ev = await a.core.createObject(g, undefined, body("Event", dinner));
    expect(await failure(a.core.createObject(g, undefined, body("Event", { ...dinner, ends: dinner.starts - 1 })))).toMatchObject({ kind: "invalid", message: "the event ends before it starts" });
    expect(await failure(a.core.createObject(g, undefined, body("Event", { ...dinner, geo: { lat: 91, lng: 0 } })))).toMatchObject({ kind: "invalid" });
    await syncAll(a, b);
    expect(fields<Event>(await b.core.readObject(g, ev))).toEqual(dinner);
    expect((await ofType(b, g, "Event")).map((o) => o.id)).toEqual([ev]);

    const later = { ...dinner, ends: dinner.ends + 3600 * 1000 };
    await b.core.editObject(g, ev, body("Event", later));
    await syncAll(b, a);
    expect(fields<Event>(await a.core.readObject(g, ev))).toEqual(later);

    // An edit that crosses start and end is invalid: refused here, never pushed.
    const crossing = await failure(a.core.editObject(g, ev, body("Event", { ...later, starts: later.ends + 1 })));
    expect(crossing).toMatchObject({ kind: "invalid", message: "the event ends before it starts" });
    expect((await a.core.readObject(g, ev))?.pending).toBe(false);
    const report = await a.core.sync();
    expect(report.invalid).toEqual([]);
    await b.core.sync();
    expect(fields<Event>(await b.core.readObject(g, ev))).toEqual(later);

    // A yearly all-day event: a birthday of someone not in the app.
    const birthday: Event = { title: "Mum", starts: 40 * DAY, ends: 41 * DAY, all_day: true, location: "", yearly: true, geo: { lat: 52.37, lng: 4.9 } };
    const bd = await a.core.createObject(g, undefined, body("Event", birthday));
    await syncAll(a, b);
    expect(fields<Event>(await b.core.readObject(g, bd))).toEqual(birthday);
    await b.core.deleteObject(g, ev);
    await syncAll(b, a);
    expect((await ofType(a, g, "Event")).map((o) => o.id)).toEqual([bd]);
    done();
  });

  it("F30 poll and vote", async () => {
    const { a, b, g, A, B, done } = await pair("Polls");
    const o1 = crypto.randomUUID();
    const o2 = crypto.randomUUID();
    const choice: Poll = { question: "Where?", kind: "Choice", options: [{ id: o1, text: "Pub", pos: 1 }, { id: o2, text: "Club", pos: 2 }], multi: false };
    const d1 = crypto.randomUUID();
    const d2 = crypto.randomUUID();
    const dates: Poll = { question: "When?", kind: "Dates", options: [{ id: d1, text: "", slot: 50 * DAY, pos: 1 }, { id: d2, text: "", slot: 51 * DAY, pos: 2 }], multi: true, closes_at: 49 * DAY };
    const poll = await a.core.createObject(g, undefined, body("Poll", choice));
    const datePoll = await a.core.createObject(g, undefined, body("Poll", dates));
    expect(await failure(a.core.createObject(g, undefined, body("Poll", { ...dates, options: [{ id: d1, text: "", pos: 1 }] })))).toMatchObject({ kind: "invalid" });
    expect(await failure(a.core.createObject(g, undefined, body("Poll", { ...choice, options: [] })))).toMatchObject({ kind: "invalid" });
    await syncAll(a, b);
    expect(fields<Poll>(await b.core.readObject(g, poll))).toEqual(choice);
    expect(fields<Poll>(await b.core.readObject(g, datePoll))).toEqual(dates);
    expect((await ofType(b, g, "Poll")).map((o) => o.id).sort()).toEqual([poll, datePoll].sort());

    // B votes: the vote's id is UUIDv5(poll, user); a changed vote is an edit.
    const vote = await b.core.createObject(g, poll, body("Vote", { answers: [{ option_id: o1, answer: "Yes" }] }));
    expect(vote).toBe(uuidV5(poll, uuidBytes(B)));
    expect(await failure(b.core.createObject(g, datePoll, body("Vote", { answers: [{ option_id: d1, answer: "Yes" }, { option_id: d1, answer: "No" }] })))).toMatchObject({ kind: "invalid" });
    await syncAll(b, a);
    expect(await a.core.readObject(g, vote)).toMatchObject({ parent_id: poll, author: B, body: { type: "Vote", fields: { answers: [{ option_id: o1, answer: "Yes" }] } } });
    await b.core.editObject(g, vote, body("Vote", { answers: [{ option_id: o2, answer: "Yes" }] }));
    await syncAll(b, a);
    expect(fields<Vote>(await a.core.readObject(g, vote)).answers).toEqual([{ option_id: o2, answer: "Yes" }]);
    expect((await ofType(a, g, "Vote")).filter((o) => o.parent_id === poll)).toHaveLength(1);
    const aVote = await a.core.createObject(g, poll, body("Vote", { answers: [{ option_id: o1, answer: "Maybe" }] }));
    expect(aVote).toBe(uuidV5(poll, uuidBytes(A)));

    // The author adds an option while B changes its vote offline: both land.
    const o3 = crypto.randomUUID();
    await a.core.editObject(g, poll, body("Poll", { ...choice, options: [...choice.options, { id: o3, text: "Home", pos: 3 }] }));
    await b.core.editObject(g, vote, body("Vote", { answers: [{ option_id: o1, answer: "No" }, { option_id: o2, answer: "Yes" }] }));
    await syncAll(a, b);
    await syncAll(a, b);
    for (const by of [a, b]) {
      expect(fields<Poll>(await by.core.readObject(g, poll)).options.map((o) => o.id)).toEqual([o1, o2, o3]);
      expect(fields<Vote>(await by.core.readObject(g, vote)).answers).toEqual([{ option_id: o1, answer: "No" }, { option_id: o2, answer: "Yes" }]);
      expect((await ofType(by, g, "Vote")).filter((o) => o.parent_id === poll)).toHaveLength(2);
    }

    await a.core.deleteObject(g, poll);
    await syncAll(a, b);
    expect(await b.core.readObject(g, vote)).toBeUndefined();
    expect(await b.core.readObject(g, aVote)).toBeUndefined();
    expect((await ofType(b, g, "Poll")).map((o) => o.id)).toEqual([datePoll]);
    done();
  });

  it("F31 message", async () => {
    const { a, b, g, A, B, done } = await pair("Chat");
    const m1 = await a.core.createObject(g, undefined, body("Message", { text: "hi" }));
    await syncAll(a, b);
    const m2 = await b.core.createObject(g, m1, body("Message", { text: "hi back", reply_to: m1 }));
    const ph = await photo(b, g, m1);
    await syncAll(b, a);
    expect(await a.core.readObject(g, m2)).toMatchObject({ parent_id: m1, author: B, body: { type: "Message", fields: { text: "hi back", reply_to: m1 } } });
    expect((await a.core.readObject(g, ph))?.parent_id).toBe(m1);
    expect((await ofType(a, g, "Message")).map((o) => o.id).sort()).toEqual([m1, m2].sort());
    const root = await a.core.readObject(g, m1);
    expect(root?.parent_id).toBeUndefined();
    expect(fields<Message>(root).reply_to).toBeUndefined();

    await a.core.editObject(g, m1, body("Message", { text: "hi, everyone" }));
    await syncAll(a, b);
    expect(await b.core.readObject(g, m1)).toMatchObject({ author: A, body: { type: "Message", fields: { text: "hi, everyone" } } });

    await b.core.deleteObject(g, m2);
    await syncAll(b, a);
    expect(await a.core.readObject(g, m2)).toBeUndefined();
    expect(await a.core.readObject(g, ph)).toBeDefined();
    await a.core.deleteObject(g, m1);
    await syncAll(a, b);
    expect(await b.core.readObject(g, ph)).toBeUndefined();
    expect(await ofType(b, g, "Message")).toEqual([]);
    done();
  });

  it("F32 venue", async () => {
    const { a, b, g, B, done } = await pair("Venues");
    const pub: Venue = { name: "The Pub", category: "Pub", address: "1 High Street", geo: { lat: 52.37, lng: 4.9 }, url: "https://pub.example", notes: "quiet on Mondays" };
    const v = await a.core.createObject(g, undefined, body("Venue", pub));
    expect(await failure(a.core.createObject(g, undefined, body("Venue", { ...pub, name: "" })))).toMatchObject({ kind: "invalid" });
    await syncAll(a, b);
    expect(fields<Venue>(await b.core.readObject(g, v))).toEqual(pub);
    expect((await ofType(b, g, "Venue")).map((o) => o.id)).toEqual([v]);
    const c = await b.core.createObject(g, v, { type: "Comment", fields: { text: "good beer" } });
    const r = await b.core.createObject(g, v, body("Reaction", { emoji: "🍺" }));
    expect(r).toBe(uuidV5(v, uuidBytes(B)));
    await syncAll(b, a);
    expect((await a.core.readObject(g, c))?.parent_id).toBe(v);
    expect((await a.core.readObject(g, r))?.parent_id).toBe(v);

    await a.core.editObject(g, v, body("Venue", { ...pub, notes: "quiet on Mondays, loud on Fridays", category: "Bar" }));
    await syncAll(a, b);
    expect(fields<Venue>(await b.core.readObject(g, v))).toMatchObject({ notes: "quiet on Mondays, loud on Fridays", category: "Bar", geo: pub.geo });

    await a.core.deleteObject(g, v);
    await syncAll(a, b);
    for (const id of [v, c, r]) expect(await b.core.readObject(g, id)).toBeUndefined();
    done();
  });

  it("F33 member profile", async () => {
    const { a, b, g, A, B, done } = await pair("Profiles");
    const mine = a.core.memberProfileId(g, A);
    expect(mine).toBe(b.core.memberProfileId(g, A));
    expect(mine).toBe(uuidV5(g, uuidBytes(A)));
    const anna: MemberProfile = { display_name: "Anna", avatar: [9, 9, 9], birthday: { month: 5, day: 17 } };
    expect(await a.core.createObject(g, undefined, body("MemberProfile", anna))).toBe(mine);
    expect(await failure(a.core.createObject(g, undefined, body("MemberProfile", { ...anna, birthday: { month: 13, day: 1 } })))).toMatchObject({ kind: "invalid" });
    const bobs = await b.core.createObject(g, undefined, body("MemberProfile", { display_name: "Bob", avatar: [] }));
    expect(bobs).toBe(a.core.memberProfileId(g, B));
    await syncAll(a, b);
    await syncAll(a, b);
    expect(fields<MemberProfile>(await b.core.readObject(g, mine))).toEqual(anna);
    expect(await a.core.readObject(g, bobs)).toMatchObject({ author: B, body: { type: "MemberProfile", fields: { display_name: "Bob", avatar: [] } } });
    expect((await ofType(a, g, "MemberProfile")).map((o) => o.id).sort()).toEqual([mine, bobs].sort());

    // Its user edits it; the other device sees the edit.
    const withYear = { ...anna, birthday: { month: 5, day: 17, year: 1990 } };
    await a.core.editObject(g, mine, body("MemberProfile", withYear));
    await syncAll(a, b);
    expect(fields<MemberProfile>(await b.core.readObject(g, mine))).toEqual(withYear);
    await b.core.deleteObject(g, bobs);
    await syncAll(b, a);
    expect(await a.core.readObject(g, bobs)).toBeUndefined();
    done();
  });

  it.skip("F33 member profile: only its user edits it", () => {
    // The core's editObject does not check the author of a per-user
    // singleton: another member's edit of a member profile is accepted and
    // pushed. Nothing in the client refuses it either.
  });

  it("F34 invite secret", async () => {
    const { A, B } = world.users();
    const a = await world.browser(A);
    const b = await world.browser(B);
    const g = await newGroup(a, "Invites");
    expect(await ofType(a, g, "InviteSecret")).toEqual([]);
    const link = await invite(a, g);
    const secrets = await ofType(a, g, "InviteSecret");
    expect(secrets).toHaveLength(1);
    const secret = fields<InviteSecret>(secrets[0]);
    expect(secret.invite_id).toMatch(/^[0-9a-f-]{36}$/);
    expect(secret.secret).toHaveLength(16);
    expect(secrets[0]).toMatchObject({ author: A, pending: false });
    expect(secrets[0].parent_id).toBeUndefined();
    // Every holder reads it, so any of them can verify a joiner; the
    // joiner itself is verified through it.
    await b.core.join(link);
    const pending = (await a.core.pendingKeyRequests()).filter((r) => r.group_id === g);
    expect(pending[0].invite_verified).toBe(true);
    await a.core.approveKeyRequest(pending[0].request_id);
    await b.core.sync();
    expect(await b.core.readObject(g, secrets[0].id)).toMatchObject({ author: A, body: { type: "InviteSecret", fields: secret } });
    // Its delete, like any object's, is seen by the other device.
    await a.core.deleteObject(g, secrets[0].id);
    await syncAll(a, b);
    expect(await ofType(b, g, "InviteSecret")).toEqual([]);
    a.dispose();
    b.dispose();
  });

  it.skip("F34 invite secret: never edited", () => {
    // Append-only is not enforced by the core's editObject (see F27), and
    // the shell has no edit path for an invite secret: there is nothing to
    // refuse and no refusal to assert.
  });

  it.skip("F35 unknown field", () => {
    // Neither the client nor the ModelServer offers a way to write an
    // object with a schema version above the client's: the core seals every
    // value at its own version, and the model server stores bytes it is
    // handed only through the core. The core's may_write rule is tested in
    // core/src/schema.rs.
  });
});
