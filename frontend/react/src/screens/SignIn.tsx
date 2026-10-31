import { signInUrl } from "../core";

/**
 * The browser flow: Kratos signs the person in with Basable and returns.
 * "Remember this browser" is decided here, before the device exists: with
 * it the device key and the store outlive the tab; without it both live in
 * memory and a reload makes this a new device.
 */
export function SignIn({ remember, onRemember, returnTo, joining }: { remember: boolean; onRemember: (remember: boolean) => void; returnTo: string; joining: boolean }) {
  return (
    <>
      <h1>deappje</h1>
      {joining && <p>You were given an invite link. Sign in first; the invite is kept and opened after.</p>}
      <p>Groups with a shared timeline, end to end encrypted: the server relays what only the members' devices can read. This browser becomes one of your devices.</p>
      <label className="check">
        <input type="checkbox" checked={remember} onChange={(e) => onRemember(e.target.checked)} />
        Remember this browser (keep its device key and its copy of your groups here)
      </label>
      <p>
        <a className="button" href={signInUrl(returnTo)}>
          Sign in with Basable
        </a>
      </p>
    </>
  );
}
