import { describe, expect, it } from "vitest";
import type { Core, ModelServer } from "../wasm/deappje_core.js";
import type { CoreModule } from "./load";
import { webRandom } from "./seams/random";
import { memorySecrets } from "./seams/secrets";
import { memoryStorage } from "./seams/storage";
import { loadCoreForTests } from "./testing";

// Two browsers of two users over the server in memory, every seam in
// memory, no network: what the shells' end-to-end test is for
// (docs/01-deappje/03-testing.md).

const ORIGIN = "https://app.example";
const USER_A = "00000000-0000-0000-0000-00000000000a";
const USER_B = "00000000-0000-0000-0000-00000000000b";

/** A browser: a core over the model server, signed in as `userId` through a token, registered as a `web` device. */
async function browser(core: CoreModule, server: ModelServer, n: number, userId: string): Promise<Core> {
  const token = `tok-${n}`;
  server.signIn(token, userId);
  const deviceKey = new core.SoftwareDeviceKey(new Uint8Array(32).fill(n));
  const clock = { nowMs: () => 1_700_000_000_000 + n };
  const c = new core.Core(server, memoryStorage(), memorySecrets(), deviceKey, webRandom, clock, ORIGIN);
  await c.setSessionToken(token);
  const identity = await c.restoreSession();
  expect(identity?.id).toBe(userId);
  expect(await c.userId()).toBe(userId);
  await c.registerDevice("web");
  return c;
}

describe("two browsers share a group through the wasm core", () => {
  it("A creates and posts, B joins through the link, A approves, B reads", async () => {
    const core = await loadCoreForTests();
    const server = new core.ModelServer(ORIGIN, new Uint8Array(32).fill(1));
    const a = await browser(core, server, 1, USER_A);
    const b = await browser(core, server, 2, USER_B);

    // A: a group, its profile, a post, an invite.
    const g = await a.createGroup();
    const profile = await a.createObject(g, undefined, { type: "GroupProfile", fields: { name: "Us", description: "", avatar: [], kind: "Group" } });
    expect(profile).toBe(a.groupProfileId(g));
    const post = await a.createObject(g, undefined, { type: "Post", fields: { text: "hello" } });
    expect((await a.readObject(g, post))?.pending).toBe(true);
    let report = await a.sync();
    expect(report.changed).toContain(g);
    expect(report.failed).toEqual([]);
    expect((await a.readObject(g, post))?.pending).toBe(false);
    const link = await a.createInvite(g, "member", 0, 0);
    expect(link).toMatch(/^\/join\/[a-z0-9]+#/);
    await a.sync();

    // B joins with the link as shared (origin and all); no key yet.
    const joined = await b.join(ORIGIN + link);
    expect(joined.group_id).toBe(g);
    const groupsOfB = await b.listGroups();
    expect(groupsOfB).toEqual([{ id: g, holds_keys: false, phase: "new" }]);
    expect(await b.listObjects(g)).toEqual([]);

    // A sees the request, the invite vouching for it, and approves.
    const pending = await a.pendingKeyRequests();
    expect(pending).toHaveLength(1);
    expect(pending[0]).toMatchObject({ group_id: g, user_id: USER_B, platform: "web", invite_verified: true });
    expect(pending[0].fingerprint).toBe(await b.fingerprint());
    expect(pending[0].device_id).toBe(await b.deviceId());
    await a.approveKeyRequest(pending[0].request_id);

    // B syncs: the key arrives and the rows with it.
    report = await b.sync();
    expect(report.keys_received).toEqual([g]);
    expect(report.changed).toContain(g);
    expect((await b.listGroups())[0].holds_keys).toBe(true);
    const seen = await b.listObjects(g);
    const posts = seen.filter((o) => o.body.type === "Post");
    expect(posts).toHaveLength(1);
    expect(posts[0]).toMatchObject({ id: post, author: USER_A, pending: false, body: { type: "Post", fields: { text: "hello" } } });
    expect(posts[0].seq).toBeGreaterThan(0);
    const profileOfB = await b.readObject(g, b.groupProfileId(g));
    expect(profileOfB?.body).toEqual({ type: "GroupProfile", fields: { name: "Us", description: "", avatar: [], kind: "Group" } });
    expect(await b.listMembers(g)).toMatchObject([{ user_id: USER_A, role: "admin" }, { user_id: USER_B, role: "member" }]);

    // B edits A's post; A deletes it; both see the end of it.
    await b.editObject(g, post, { type: "Post", fields: { text: "hello!" } });
    await b.sync();
    await a.sync();
    expect((await a.readObject(g, post))?.body).toEqual({ type: "Post", fields: { text: "hello!" } });
    await a.deleteObject(g, post);
    await a.sync();
    await b.sync();
    expect(await b.readObject(g, post)).toBeUndefined();

    // A failure is a CoreError with its kind.
    await expect(a.createObject(g, undefined, { type: "Comment", fields: { text: "x" } })).rejects.toMatchObject({ name: "CoreError", kind: "invalid" });
    await expect(a.createObject("not-an-id", undefined, { type: "Post", fields: { text: "x" } })).rejects.toMatchObject({ kind: "invalid" });
    server.setOutage(true);
    await expect(a.sync()).rejects.toMatchObject({ kind: "api", code: "unavailable" });
    server.setOutage(false);

    // A note, for the notes path through the bindings.
    const note = await a.createObject(g, undefined, { type: "Note", fields: { title: "List" } });
    expect(await a.saveNote(g, note, "milk\n")).toBe(true);
    await a.sync();
    await b.sync();
    expect(await b.readNote(g, note)).toBe("milk\n");

    a.free();
    b.free();
    server.free();
  });
});
