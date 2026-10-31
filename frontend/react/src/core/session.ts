// How the browser's session reaches the core. The core keeps a session
// token and sends it as `x-session-token`; a browser signed in through
// Kratos's browser flow holds a cookie it cannot read instead. So the web
// shell gives the core this marker as its token: the core then asks whoami
// and sends the API calls as a signed-in device, and the transport drops
// the marker's header so the cookie is what authenticates.
export const SESSION_TOKEN_HEADER = "x-session-token";
export const COOKIE_SESSION = "cookie";

const REMEMBER = "deappje.remember";

/** Whether this browser was asked to keep its device key and store. */
export function isRemembered(store: Pick<Storage, "getItem"> = localStorage): boolean {
  try {
    return store.getItem(REMEMBER) === "1";
  } catch {
    return false;
  }
}

export function setRemembered(remember: boolean, store: Pick<Storage, "setItem" | "removeItem"> = localStorage): void {
  if (remember) store.setItem(REMEMBER, "1");
  else store.removeItem(REMEMBER);
}

/** Where "Sign in with Basable" goes: Kratos's browser flow, back to `returnTo` after. */
export function signInUrl(returnTo: string): string {
  return `/.ory/self-service/login/browser?return_to=${encodeURIComponent(returnTo)}`;
}

/** Where signing out goes: Kratos ends the cookie's session and returns to the start. */
export const SIGN_OUT_URL = "/.ory/self-service/logout/browser";
