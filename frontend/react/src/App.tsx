import { useEffect, useState } from "react";
import { Auth } from "./Auth";
import { type Client, COOKIE_SESSION, type CoreModule, describeError, type Identity, isRemembered, openClient, setRemembered } from "./core";
import { useLinkClick, useLocation, navigate } from "./router";
import { Approvals } from "./screens/Approvals";
import { Groups } from "./screens/Groups";
import { Join } from "./screens/Join";
import { SignIn } from "./screens/SignIn";
import { ThisBrowser } from "./screens/ThisBrowser";
import { Timeline } from "./screens/Timeline";
import { useSync } from "./sync";

/** The core, opened, and who it is signed in as (nobody: the sign-in screen). */
export interface Session {
  client: Client;
  identity: Identity | undefined;
}

const PENDING_JOIN = "deappje.pending-join";

/**
 * Opens the core as this browser and asks who the cookie signs in as. The
 * browser's session is a cookie the core cannot read, so the core gets the
 * marker and its transport sends the cookie (core/session.ts); a browser
 * that is signed in registers itself as a `web` device, a repeat refreshes.
 */
async function open(module: CoreModule, remember: boolean): Promise<Session> {
  const client = await openClient(module, { remember, origin: window.location.origin });
  await client.core.setSessionToken(COOKIE_SESSION);
  const identity = await client.core.restoreSession();
  if (identity) await client.core.registerDevice("web");
  return { client, identity };
}

export function App({ module }: { module: CoreModule }) {
  const [remember, setRemember] = useState(isRemembered);
  const [session, setSession] = useState<Session | undefined>();
  const [failure, setFailure] = useState<string | undefined>();
  const location = useLocation();
  const onLink = useLinkClick();

  useEffect(() => {
    let cancelled = false;
    let opened: Client | undefined;
    setSession(undefined);
    open(module, remember).then(
      (s) => {
        if (cancelled) {
          s.client.dispose();
          return;
        }
        opened = s.client;
        setSession(s);
        // A join link opened while signed out continues after the sign-in.
        const pending = sessionStorage.getItem(PENDING_JOIN);
        if (s.identity && pending) {
          sessionStorage.removeItem(PENDING_JOIN);
          navigate(pending, true);
        }
      },
      (e: unknown) => !cancelled && setFailure(describeError(e)),
    );
    return () => {
      cancelled = true;
      opened?.dispose();
    };
  }, [module, remember]);

  const signedIn = session?.identity !== undefined;
  const sync = useSync(session?.client, signedIn);
  // A join link opened while signed out is kept for after the sign-in.
  const joining = location.pathname.startsWith("/join/");
  useEffect(() => {
    if (session && !session.identity && joining) sessionStorage.setItem(PENDING_JOIN, location.pathname + location.hash);
  }, [session, joining, location]);

  const chooseRemember = (value: boolean) => {
    setRemembered(value);
    setRemember(value);
  };

  if (location.pathname.startsWith("/auth/")) {
    return (
      <Shell signedIn={false} onLink={onLink}>
        <Auth />
      </Shell>
    );
  }
  if (failure) {
    return (
      <Shell signedIn={false} onLink={onLink}>
        <h1>deappje</h1>
        <p className="notice-error">{failure}</p>
      </Shell>
    );
  }
  if (!session) {
    return (
      <Shell signedIn={false} onLink={onLink}>
        <p className="notice">Opening…</p>
      </Shell>
    );
  }
  if (!session.identity) {
    return (
      <Shell signedIn={false} onLink={onLink}>
        <SignIn remember={remember} onRemember={chooseRemember} returnTo={joining ? window.location.origin + "/" : window.location.href} joining={joining} />
      </Shell>
    );
  }

  const { client, identity } = session;
  const groupMatch = /^\/g\/([0-9a-f-]{36})$/.exec(location.pathname);
  let screen;
  if (groupMatch) screen = <Timeline client={client} groupId={groupMatch[1]} userId={identity.id} sync={sync} />;
  else if (location.pathname === "/approvals") screen = <Approvals client={client} sync={sync} />;
  else if (location.pathname === "/browser") screen = <ThisBrowser client={client} identity={identity} remember={remember} onRemember={chooseRemember} />;
  else if (location.pathname.startsWith("/join/")) screen = <Join client={client} link={location.pathname + location.hash} sync={sync} />;
  else screen = <Groups client={client} sync={sync} />;

  return (
    <Shell signedIn onLink={onLink} syncing={sync.syncing} syncError={sync.error}>
      {screen}
    </Shell>
  );
}

function Shell({ children, signedIn, onLink, syncing, syncError }: { children: React.ReactNode; signedIn: boolean; onLink: ReturnType<typeof useLinkClick>; syncing?: boolean; syncError?: string }) {
  return (
    <>
      <header className="topbar">
        <a className="brand" href="/" onClick={onLink}>
          deappje
        </a>
        <nav>
          {signedIn && (
            <>
              <a href="/" onClick={onLink}>
                Groups
              </a>
              <a href="/approvals" onClick={onLink}>
                Approvals
              </a>
              <a href="/browser" onClick={onLink}>
                This browser
              </a>
              <span className="sync" title={syncError}>
                {syncing ? "syncing…" : syncError ? "sync failed" : ""}
              </span>
            </>
          )}
        </nav>
      </header>
      <main className="panel">{children}</main>
    </>
  );
}
