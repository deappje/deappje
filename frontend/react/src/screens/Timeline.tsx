import { type FormEvent, useCallback, useEffect, useState } from "react";
import { type Client, describeError, type GroupView, type ObjectView } from "../core";
import type { SyncState } from "../sync";
import { groupName } from "./Groups";

type Post = ObjectView & { body: { type: "Post"; fields: { text: string } } };

function isPost(o: ObjectView): o is Post {
  return o.body.type === "Post";
}

function when(ms: number): string {
  return new Date(ms).toLocaleString();
}

/** A group's timeline: its posts, newest first; compose, edit and delete one's own. */
export function Timeline({ client, groupId, userId, sync }: { client: Client; groupId: string; userId: string; sync: SyncState }) {
  const [group, setGroup] = useState<GroupView | undefined>();
  const [name, setName] = useState<string | undefined>();
  const [posts, setPosts] = useState<Post[]>([]);
  const [text, setText] = useState("");
  const [editing, setEditing] = useState<{ id: string; text: string } | undefined>();
  const [invite, setInvite] = useState<string | undefined>();
  const [error, setError] = useState<string | undefined>();
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const view = (await client.core.listGroups()).find((g) => g.id === groupId);
      setGroup(view);
      setName(view ? await groupName(client, view) : undefined);
      const objects = await client.core.listObjects(groupId);
      setPosts(objects.filter(isPost).sort((a, b) => b.authored_at_ms - a.authored_at_ms));
      setError(undefined);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client, groupId]);

  useEffect(() => {
    void load();
  }, [load, sync.tick]);

  const act = async (work: () => Promise<void>) => {
    setBusy(true);
    try {
      await work();
      setError(undefined);
      await load();
      await sync.syncNow();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  const compose = (e: FormEvent) => {
    e.preventDefault();
    const trimmed = text.trim();
    if (!trimmed) return;
    void act(async () => {
      await client.core.createObject(groupId, undefined, { type: "Post", fields: { text: trimmed } });
      setText("");
    });
  };

  const saveEdit = (e: FormEvent) => {
    e.preventDefault();
    if (!editing) return;
    const { id, text } = editing;
    void act(async () => {
      await client.core.editObject(groupId, id, { type: "Post", fields: { text: text.trim() } });
      setEditing(undefined);
    });
  };

  const remove = (id: string) => void act(() => client.core.deleteObject(groupId, id));

  const makeInvite = () =>
    void act(async () => {
      const link = await client.core.createInvite(groupId, "member", 0, 0);
      setInvite(window.location.origin + link);
    });

  if (group && !group.holds_keys) {
    return (
      <>
        <h1>Waiting for approval</h1>
        <p>This browser holds no key of this group yet. A member who does sees its request on their Approvals page; the invite link vouches for it, or they compare this browser's fingerprint.</p>
        {error && <p className="notice-error">{error}</p>}
      </>
    );
  }

  return (
    <>
      <h1>{name ?? "Timeline"}</h1>
      {error && <p className="notice-error">{error}</p>}
      <p className="actions">
        <button type="button" onClick={makeInvite} disabled={busy}>
          Invite someone
        </button>
        {invite && (
          <span className="invite">
            <code>{invite}</code> (share it; the part after # never reaches the server)
          </span>
        )}
      </p>
      <form className="compose" onSubmit={compose}>
        <textarea value={text} onChange={(e) => setText(e.target.value)} placeholder="Write a post" rows={3} disabled={busy} />
        <button type="submit" disabled={busy || !text.trim()}>
          Post
        </button>
      </form>
      {posts.length === 0 ? (
        <p className="notice">Nothing posted yet.</p>
      ) : (
        <ul className="posts">
          {posts.map((p) => (
            <li key={p.id} className={p.pending ? "pending" : ""}>
              {editing?.id === p.id ? (
                <form className="compose" onSubmit={saveEdit}>
                  <textarea value={editing.text} onChange={(e) => setEditing({ id: p.id, text: e.target.value })} rows={3} autoFocus />
                  <span className="actions">
                    <button type="submit" disabled={busy || !editing.text.trim()}>
                      Save
                    </button>
                    <button type="button" onClick={() => setEditing(undefined)}>
                      Cancel
                    </button>
                  </span>
                </form>
              ) : (
                <>
                  <p className="text">{p.body.fields.text}</p>
                  <p className="meta">
                    {when(p.authored_at_ms)}
                    {p.edited_at_ms !== p.authored_at_ms && " (edited)"}
                    {p.pending && " · sending"}
                    {p.author === userId && (
                      <>
                        {" · "}
                        <button type="button" className="link" onClick={() => setEditing({ id: p.id, text: p.body.fields.text })} disabled={busy}>
                          Edit
                        </button>
                        {" · "}
                        <button type="button" className="link" onClick={() => remove(p.id)} disabled={busy}>
                          Delete
                        </button>
                      </>
                    )}
                  </p>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}
