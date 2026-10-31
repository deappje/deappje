import type { TransportHeader, TransportSeam } from "../../wasm/deappje_core.js";
import { COOKIE_SESSION, SESSION_TOKEN_HEADER } from "../session";

/**
 * HTTP through `fetch`. Same-origin requests (the API, Kratos) carry the
 * browser's cookies, which is how the session travels: the core's session
 * token on the web is the marker `COOKIE_SESSION`, and its header is
 * dropped here so Kratos and the API read the cookie instead. A presigned
 * bucket URL is another origin and goes without credentials.
 */
export function fetchTransport(origin: string, fetchFn: typeof fetch = (...args) => fetch(...args)): TransportSeam {
  return {
    async send(request) {
      const headers = new Headers();
      for (const h of request.headers) {
        if (h.name === SESSION_TOKEN_HEADER && h.value === COOKIE_SESSION) continue;
        headers.set(h.name, h.value);
      }
      const sameOrigin = request.url.startsWith(origin + "/") || request.url === origin;
      const response = await fetchFn(request.url, {
        method: request.method,
        headers,
        body: request.body.length > 0 ? new Uint8Array(request.body) : undefined,
        credentials: sameOrigin ? "include" : "omit",
      });
      const out: TransportHeader[] = [];
      response.headers.forEach((value, name) => out.push({ name, value }));
      return { status: response.status, headers: out, body: new Uint8Array(await response.arrayBuffer()) };
    },
  };
}
