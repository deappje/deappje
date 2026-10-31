import { type FormEvent, useCallback, useEffect, useState } from "react";
import { type Client, describeError, type GroupView } from "../core";
import { useLinkClick } from "../router";
import type { SyncState } from "../sync";

interface GroupRow extends GroupView {
  /** The profile's name, when the device can read it. */
  name: string | undefined;
}

/** The group's profile, the first object in every group. */
export async function groupName(client: Client, group: GroupView): Promise<string | undefined> {
  if (!group.holds_keys) return undefined;
  const profile = await client.core.readObject(group.id, client.core.groupProfileId(group.id));
  return profile?.body.type === "GroupProfile" ? profile.body.fields.name : undefined;
}

export function Groups({ client, sync }: { client: Client; sync: SyncState }) {
  const [groups, setGroups] = useState<GroupRow[] | undefined>();
  const [error, setError] = useState<string | undefined>();
  const [name, setName] = useState("");
  const [link, setLink] = useState("");
  const [busy, setBusy] = useState(false);
  const onLink = useLinkClick();

  const load = useCallback(async () => {
    try {
      const views = await client.core.listGroups();
      setGroups(await Promise.all(views.map(async (g) => ({ ...g, name: await groupName(client, g) }))));
      setError(undefined);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useEffect(() => {
    void load();
  }, [load, sync.tick]);

  const act = async (work: () => Promise<void>) => {
    setBusy(true);
    try {
      await work();
      setError(undefined);
      await sync.syncNow();
      await load();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  const create = (e: FormEvent) => {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) return;
    void act(async () => {
      const id = await client.core.createGroup();
      await client.core.createObject(id, undefined, { type: "GroupProfile", fields: { name: trimmed, description: "", avatar: [], kind: "Group" } });
      setName("");
    });
  };

  const join = (e: FormEvent) => {
    e.preventDefault();
    const trimmed = link.trim();
    if (!trimmed) return;
    void act(async () => {
      await client.core.join(trimmed);
      setLink("");
    });
  };

  return (
    <>
      <h1>Groups</h1>
      {error && <p className="notice-error">{error}</p>}
      {groups === undefined ? (
        <p className="notice">Loading…</p>
      ) : groups.length === 0 ? (
        <p className="notice">No groups yet. Create one, or paste an invite link.</p>
      ) : (
        <ul className="list">
          {groups.map((g) => (
            <li key={g.id}>
              <a href={`/g/${g.id}`} onClick={onLink}>
                {g.name ?? <span className="notice">{g.holds_keys ? "Unnamed group" : "Waiting for a member to approve this browser"}</span>}
              </a>
              <span className="meta">{g.phase === "holder" ? "" : g.phase === "catching_up" ? "catching up" : g.holds_keys ? "new" : "no key yet"}</span>
            </li>
          ))}
        </ul>
      )}
      <form className="row" onSubmit={create}>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="A new group's name" disabled={busy} />
        <button type="submit" disabled={busy || !name.trim()}>
          Create
        </button>
      </form>
      <form className="row" onSubmit={join}>
        <input value={link} onChange={(e) => setLink(e.target.value)} placeholder="Paste an invite link to join" disabled={busy} />
        <button type="submit" disabled={busy || !link.trim()}>
          Join
        </button>
      </form>
    </>
  );
}
