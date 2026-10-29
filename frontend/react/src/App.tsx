import { useEffect, useState } from "react";
import { health } from "./api";
import { Auth } from "./Auth";

export function App() {
  const [status, setStatus] = useState("Checking the API…");
  const [signedIn, setSignedIn] = useState<boolean | null>(null);

  useEffect(() => {
    health.check({}).then(
      (res) => setStatus(res.ready ? "The API is ready." : "The API is starting…"),
      () => setStatus("The API is not reachable yet."),
    );
    // The Kratos session, read with the browser's cookie.
    fetch("/.ory/sessions/whoami", { credentials: "include" }).then(
      (res) => setSignedIn(res.ok),
      () => setSignedIn(false),
    );
  }, []);

  return (
    <>
      <header className="topbar">
        <a className="brand" href="/">
          deappje
        </a>
        <nav>
          {signedIn === false && <a href="/.ory/self-service/login/browser">Sign in</a>}
          {signedIn === true && <a href="/.ory/self-service/logout/browser">Sign out</a>}
        </nav>
      </header>
      <main className="panel">
        {window.location.pathname.startsWith("/auth/") ? (
          <Auth />
        ) : (
          <>
            <h1>deappje</h1>
            <p>
              The application is running. This page is a React app served from the environment's public bucket; the API answers under{" "}
              <code>/api/</code> and sessions come from Kratos under <code>/.ory/</code>.
            </p>
            <p>{status}</p>
          </>
        )}
      </main>
    </>
  );
}
