import { useCallback, useEffect, useState } from "react";
import { type Client, describeError, type PendingRequestView } from "../core";
import type { SyncState } from "../sync";

/**
 * The key requests this browser may approve: another device of a member
 * (or of oneself) asking for a group's keys. An invite vouches for one;
 * otherwise the person compares the fingerprint the other device shows.
 */
export function Approvals({ client, sync }: { client: Client; sync: SyncState }) {
  const [pending, setPending] = useState<PendingRequestView[] | undefined>();
  const [error, setError] = useState<string | undefined>();
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setPending(await client.core.pendingKeyRequests());
      setError(undefined);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useEffect(() => {
    void load();
  }, [load, sync.tick]);

  const approve = async (id: string) => {
    setBusy(true);
    try {
      await client.core.approveKeyRequest(id);
      await load();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <h1>Approvals</h1>
      {error && <p className="notice-error">{error}</p>}
      {pending === undefined ? (
        <p className="notice">Loading…</p>
      ) : pending.length === 0 ? (
        <p className="notice">No device is waiting for keys.</p>
      ) : (
        <ul className="list">
          {pending.map((r) => (
            <li key={r.request_id} className="request">
              <div>
                <strong>{r.platform}</strong> device of {r.user_id}
                <br />
                <code className="fingerprint">{r.fingerprint}</code>
                <br />
                <span className="meta">
                  {r.invite_verified === true && "The invite link vouches for this device."}
                  {r.invite_verified === false && "Its invite tag does not match: approve only after comparing the fingerprint."}
                  {r.invite_verified === undefined && "No invite: compare the fingerprint with what the device shows."}
                </span>
              </div>
              <button type="button" onClick={() => approve(r.request_id)} disabled={busy}>
                Approve
              </button>
            </li>
          ))}
        </ul>
      )}
    </>
  );
}
