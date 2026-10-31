import { useState } from "react";
import { type Client, describeError } from "../core";
import { navigate } from "../router";
import type { SyncState } from "../sync";

/** An invite link opened in this browser: `/join/<code>#<secret>`. */
export function Join({ client, link, sync }: { client: Client; link: string; sync: SyncState }) {
  const [error, setError] = useState<string | undefined>();
  const [busy, setBusy] = useState(false);

  const join = async () => {
    setBusy(true);
    try {
      const result = await client.core.join(link);
      await sync.syncNow();
      navigate(`/g/${result.group_id}`, true);
    } catch (e) {
      setError(describeError(e));
      setBusy(false);
    }
  };

  return (
    <>
      <h1>Join a group</h1>
      <p>You were invited. Joining accepts the invite and asks the members for the group's keys; a member approves this browser from their Approvals page, and the group opens at the next sync.</p>
      {error && <p className="notice-error">{error}</p>}
      <p>
        <button type="button" onClick={join} disabled={busy}>
          Join
        </button>
      </p>
    </>
  );
}
