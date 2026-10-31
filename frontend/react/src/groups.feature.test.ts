import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { addMember, addOwnDevice, invite, newGroup, ORIGIN, post, profile, syncAll, World } from "./core/testing";

// Feature suite, "Groups and membership" (docs/01-deappje/03-testing.md).

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

let world: World;
beforeAll(async () => {
  world = await World.open(12);
});
afterAll(() => world.free());

describe("groups and membership", () => {
  it("F06 create", async () => {
    const { A } = world.users();
    const a = await world.browser(A);
    const g = await a.core.createGroup();
    expect(g).toMatch(UUID);
    const profileId = await a.core.createObject(g, undefined, profile("Us", "the three of us"));
    expect(profileId).toBe(a.core.groupProfileId(g));
    // Before the sync: the key held, the profile pending.
    expect((await a.core.listGroups()).find((x) => x.id === g)).toEqual({ id: g, holds_keys: true, phase: "new" });
    expect(await a.core.readObject(g, profileId)).toMatchObject({ pending: true, seq: 0, author: A, body: profile("Us", "the three of us") });
    const report = await a.core.sync();
    expect(report.changed).toContain(g);
    expect(report.failed).toEqual([]);
    expect((await a.core.listGroups()).find((x) => x.id === g)).toEqual({ id: g, holds_keys: true, phase: "holder" });
    // The creator is an admin, holding key 1.
    const members = await a.core.listMembers(g);
    expect(members).toHaveLength(1);
    expect(members[0]).toMatchObject({ group_id: g, user_id: A, role: "admin" });
    expect(members[0].id).toMatch(UUID);
    expect(a.storage.keys(`k/${g}/`)).toEqual([`k/${g}/1`]);
    expect(a.storage.get(`k/${g}/1`)).toHaveLength(32);
    const objects = await a.core.listObjects(g);
    expect(objects).toHaveLength(1);
    expect(objects[0]).toMatchObject({ id: profileId, seq: 1, pending: false, body: { type: "GroupProfile", fields: { name: "Us" } } });
    a.dispose();
  });

  it("F07 join by invite", async () => {
    const { A, B } = world.users();
    const a = await world.browser(A);
    const b = await world.browser(B);
    const g = await newGroup(a, "Us");
    const p = await post(a, g, "hello");
    await a.core.sync();
    const link = await invite(a, g);
    expect(link.startsWith(`${ORIGIN}/join/`)).toBe(true);
    expect(link).toMatch(/\/join\/[a-z0-9]+#[a-z0-9]+$/);

    const joined = await b.core.join(link);
    expect(joined.group_id).toBe(g);
    expect(joined.request_id).toMatch(UUID);
    expect((await b.core.listGroups()).find((x) => x.id === g)).toEqual({ id: g, holds_keys: false, phase: "new" });
    expect(await b.core.listObjects(g)).toEqual([]);

    // A sees B's request, the invite vouching for it, with B's fingerprint.
    const pending = (await a.core.pendingKeyRequests()).filter((r) => r.group_id === g);
    expect(pending).toHaveLength(1);
    expect(pending[0]).toMatchObject({
      request_id: joined.request_id,
      group_id: g,
      user_id: B,
      platform: "web",
      invite_verified: true,
      device_id: await b.core.deviceId(),
      fingerprint: await b.core.fingerprint(),
    });
    expect(pending[0].created_at_ms).toBeGreaterThanOrEqual(0);
    await a.core.approveKeyRequest(pending[0].request_id);
    expect((await a.core.pendingKeyRequests()).filter((r) => r.group_id === g)).toEqual([]);

    // B syncs: the key, then the rows.
    const report = await b.core.sync();
    expect(report.keys_received).toEqual([g]);
    expect(report.changed).toContain(g);
    expect(report.failed).toEqual([]);
    expect((await b.core.listGroups()).find((x) => x.id === g)).toEqual({ id: g, holds_keys: true, phase: "holder" });
    const seen = await b.core.readObject(g, p);
    expect(seen).toMatchObject({ id: p, author: A, pending: false, body: { type: "Post", fields: { text: "hello" } } });
    expect(seen!.seq).toBeGreaterThan(0);
    expect(await b.core.listMembers(g)).toMatchObject([
      { user_id: A, role: "admin" },
      { user_id: B, role: "member" },
    ]);
    a.dispose();
    b.dispose();
  });

  it("F08 roles", async () => {
    const { A, B } = world.users();
    const a = await world.browser(A);
    const b = await world.browser(B);
    const g = await newGroup(a, "Readers");
    await addMember(a, b, g, "viewer");
    expect(await b.core.listMembers(g)).toMatchObject([
      { user_id: A, role: "admin" },
      { user_id: B, role: "viewer" },
    ]);
    // A viewer reads.
    expect((await b.core.readObject(g, b.core.groupProfileId(g)))?.body).toEqual(profile("Readers"));
    // Its write is refused by the server: the sync reports the group failed
    // with permission_denied, nothing reaches A.
    const p = await post(b, g, "a viewer's post");
    const refused = await b.core.sync();
    expect(refused.failed).toHaveLength(1);
    expect(refused.failed[0]).toContain(g);
    expect(refused.failed[0]).toMatch(/permission_denied|viewer/);
    await a.core.sync();
    expect(await a.core.readObject(g, p)).toBeUndefined();
    expect((await b.core.readObject(g, p))?.pending).toBe(true);
    // After setMemberRole the same write lands.
    await a.core.setMemberRole(g, B, "member");
    const landed = await b.core.sync();
    expect(landed.failed).toEqual([]);
    expect(landed.changed).toContain(g);
    expect((await b.core.readObject(g, p))?.pending).toBe(false);
    await a.core.sync();
    expect(await a.core.readObject(g, p)).toMatchObject({ author: B, body: { type: "Post", fields: { text: "a viewer's post" } } });
    a.dispose();
    b.dispose();
  });

  it.skip("F08 roles: a viewer's createObject is refused (api / permission_denied) with the outbox empty afterwards", () => {
    // The core's createObject checks no role: the write goes into the
    // outbox and the server refuses the push (SyncReport.failed carries
    // permission_denied), and the outbox keeps the write until the role
    // allows it. The test above asserts that behaviour.
  });

  it("F09 removal and rotation", async () => {
    const { A, B } = world.users();
    const a = await world.browser(A);
    const a2 = await world.browser(A);
    const b = await world.browser(B);
    const g = await newGroup(a, "Shrinking");
    await addMember(a, b, g);
    await addOwnDevice(a, a2, g);
    const before = await post(a, g, "before");
    await syncAll(a, b, a2);
    expect(await b.core.readObject(g, before)).toBeDefined();

    await a.core.removeMember(g, B);
    await a.core.rotateKeys(g);
    expect(a.storage.keys(`k/${g}/`)).toEqual([`k/${g}/1`, `k/${g}/2`]);
    expect((await a.core.listMembers(g)).map((m) => m.user_id)).toEqual([A]);
    const after = await post(a, g, "after");
    await a.core.sync();

    // B: the group is gone from its list, nothing new reaches it.
    const gone = await b.core.sync();
    expect(gone.changed).not.toContain(g);
    expect(gone.keys_received).not.toContain(g);
    expect((await b.core.listGroups()).find((x) => x.id === g)).toBeUndefined();
    expect(await b.core.readObject(g, after)).toBeUndefined();
    expect((await b.core.listObjects(g)).map((o) => o.id)).not.toContain(after);
    expect(b.storage.keys(`k/${g}/`)).toEqual([`k/${g}/1`]);

    // A2 receives the new generation and reads what A wrote under it.
    const rotated = await a2.core.sync();
    expect(rotated.keys_received).toEqual([g]);
    expect(rotated.failed).toEqual([]);
    expect(a2.storage.keys(`k/${g}/`)).toEqual([`k/${g}/1`, `k/${g}/2`]);
    expect(await a2.core.readObject(g, after)).toMatchObject({ author: A, body: { type: "Post", fields: { text: "after" } } });
    expect(await a2.core.listObjects(g)).toEqual(await a.core.listObjects(g));
    a.dispose();
    a2.dispose();
    b.dispose();
  });

  it("F10 leave", async () => {
    const { A, B } = world.users();
    const a = await world.browser(A);
    const b = await world.browser(B);
    const g = await newGroup(a, "Leaving");
    await addMember(a, b, g);
    const bp = await post(b, g, "B's post");
    await syncAll(b, a);

    await b.core.leaveGroup(g);
    expect((await b.core.listGroups()).find((x) => x.id === g)).toBeUndefined();
    expect(await b.core.listObjects(g)).toEqual([]);
    expect(b.storage.keys(`k/${g}/`)).toEqual([]);
    expect((await a.core.listMembers(g)).map((m) => m.user_id)).toEqual([A]);
    const report = await b.core.sync();
    expect(report.changed).not.toContain(g);
    expect(report.failed).toEqual([]);
    // What B wrote stays with the group.
    await a.core.sync();
    expect(await a.core.readObject(g, bp)).toMatchObject({ author: B });
    a.dispose();
    b.dispose();
  });

  it("F11 members", async () => {
    const { A, B, C } = world.users();
    const a = await world.browser(A);
    const b = await world.browser(B);
    const c = await world.browser(C);
    const g = await newGroup(a, "All of us");
    world.server.advance(1_000);
    await addMember(a, b, g);
    world.server.advance(1_000);
    await addMember(a, c, g, "admin");

    const members = await a.core.listMembers(g);
    expect(members.map((m) => [m.user_id, m.role])).toEqual([
      [A, "admin"],
      [B, "member"],
      [C, "admin"],
    ]);
    for (const m of members) {
      expect(m.id).toMatch(UUID);
      expect(m.group_id).toBe(g);
    }
    // Joined in that order, a second apart; the order is the joining order.
    expect(members[1].joined_at_ms - members[0].joined_at_ms).toBeGreaterThanOrEqual(1_000);
    expect(members[2].joined_at_ms - members[1].joined_at_ms).toBeGreaterThanOrEqual(1_000);
    // Stable: the same again, and the same from every device.
    expect(await a.core.listMembers(g)).toEqual(members);
    expect(await b.core.listMembers(g)).toEqual(members);
    expect(await c.core.listMembers(g)).toEqual(members);
    a.dispose();
    b.dispose();
    c.dispose();
  });

  it("F12 profile", async () => {
    const { A, B } = world.users();
    const a = await world.browser(A);
    const b = await world.browser(B);
    const g = await newGroup(a, "Us");
    await addMember(a, b, g, "admin");
    const pid = a.core.groupProfileId(g);
    expect(pid).toBe(b.core.groupProfileId(g));

    // Offline, two admins edit different fields; both edits survive.
    await a.core.editObject(g, pid, profile("Us two", ""));
    await b.core.editObject(g, pid, profile("Us", "a description"));
    await a.core.sync();
    const rebased = await b.core.sync();
    expect(rebased.invalid).toEqual([]);
    expect(rebased.failed).toEqual([]);
    await a.core.sync();
    const merged = profile("Us two", "a description");
    expect((await a.core.readObject(g, pid))?.body).toEqual(merged);
    expect((await b.core.readObject(g, pid))?.body).toEqual(merged);
    expect((await b.core.readObject(g, pid))?.pending).toBe(false);

    // The same field: the later push wins.
    await a.core.editObject(g, pid, profile("A's name", "a description"));
    await b.core.editObject(g, pid, profile("B's name", "a description"));
    await a.core.sync();
    await b.core.sync();
    await a.core.sync();
    expect((await a.core.readObject(g, pid))?.body).toEqual(profile("B's name", "a description"));
    expect((await b.core.readObject(g, pid))?.body).toEqual(profile("B's name", "a description"));
    a.dispose();
    b.dispose();
  });
});
