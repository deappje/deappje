import { useEffect, useState } from "react";
import { type Client, describeError, hasWebCryptoX25519, type Identity, SIGN_OUT_URL } from "../core";

/**
 * The device this browser is: its fingerprint large, for a phone to read
 * off the screen at an approval, and how its key and store are kept.
 */
export function ThisBrowser({ client, identity, remember, onRemember }: { client: Client; identity: Identity; remember: boolean; onRemember: (remember: boolean) => void }) {
  const [fingerprint, setFingerprint] = useState("");
  const [deviceId, setDeviceId] = useState("");
  const [webCrypto, setWebCrypto] = useState<boolean | undefined>();
  const [error, setError] = useState<string | undefined>();

  useEffect(() => {
    Promise.all([client.core.fingerprint(), client.core.deviceId(), hasWebCryptoX25519()]).then(
      ([f, d, w]) => {
        setFingerprint(f);
        setDeviceId(d);
        setWebCrypto(w);
      },
      (e: unknown) => setError(describeError(e)),
    );
  }, [client]);

  const signOut = async () => {
    try {
      await client.core.logout();
    } finally {
      window.location.assign(SIGN_OUT_URL);
    }
  };

  return (
    <>
      <h1>This browser</h1>
      {error && <p className="notice-error">{error}</p>}
      <p className="notice">Another device approving this one compares this fingerprint with what it is shown.</p>
      <p className="fingerprint-large">{fingerprint || "…"}</p>
      <dl className="facts">
        <dt>Signed in as</dt>
        <dd>{identity.email ?? identity.id}</dd>
        <dt>Device id</dt>
        <dd>
          <code>{deviceId}</code>
        </dd>
        <dt>Device key</dt>
        <dd>
          X25519 in software, {client.deviceKey === "software-indexeddb" ? "kept in this browser's storage" : "in memory for this tab"}.{" "}
          {webCrypto === true && "This browser could hold it as a non-extractable WebCrypto key; the core's key seam has to become asynchronous first."}
          {webCrypto === false && "This browser has no WebCrypto X25519."}
        </dd>
        <dt>Local store</dt>
        <dd>{client.storage === "indexeddb" ? "IndexedDB, read into memory at start, written through." : "Memory: gone when the tab closes."}</dd>
      </dl>
      <label className="check">
        <input type="checkbox" checked={remember} onChange={(e) => onRemember(e.target.checked)} />
        Remember this browser
      </label>
      <p className="notice">Changing it opens the core again with another key, so the groups see a new device that asks for keys once more.</p>
      <p>
        <button type="button" onClick={signOut}>
          Sign out
        </button>
      </p>
    </>
  );
}
