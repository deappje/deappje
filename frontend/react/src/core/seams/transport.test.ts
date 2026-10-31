import { describe, expect, it } from "vitest";
import { COOKIE_SESSION, SESSION_TOKEN_HEADER } from "../session";
import { fetchTransport } from "./transport";

const ORIGIN = "https://app.example";

function fakeFetch(answer: (url: string, init: RequestInit) => Response) {
  const calls: { url: string; init: RequestInit }[] = [];
  const fetchFn: typeof fetch = async (input, init) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
    calls.push({ url, init: init ?? {} });
    return answer(url, init ?? {});
  };
  return { calls, fetchFn };
}

describe("fetchTransport", () => {
  it("sends the cookie for the origin and drops the session marker's header", async () => {
    const { calls, fetchFn } = fakeFetch(() => new Response(Uint8Array.from([1, 2]), { status: 200, headers: { "Content-Type": "application/proto" } }));
    const t = fetchTransport(ORIGIN, fetchFn);
    const response = await t.send({
      method: "POST",
      url: `${ORIGIN}/api/group.v1.GroupService/ListGroups`,
      headers: [
        { name: "content-type", value: "application/proto" },
        { name: SESSION_TOKEN_HEADER, value: COOKIE_SESSION },
      ],
      body: Uint8Array.from([7]),
    });
    expect(calls).toHaveLength(1);
    const init = calls[0].init;
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("include");
    const sent = init.headers as Headers;
    expect(sent.get("content-type")).toBe("application/proto");
    expect(sent.has(SESSION_TOKEN_HEADER)).toBe(false);
    expect(new Uint8Array(init.body as Uint8Array)).toEqual(Uint8Array.from([7]));
    expect(response.status).toBe(200);
    expect(response.body).toEqual(Uint8Array.from([1, 2]));
    expect(response.headers).toContainEqual({ name: "content-type", value: "application/proto" });
  });

  it("keeps a real session token and sends nothing but the request to another origin", async () => {
    const { calls, fetchFn } = fakeFetch(() => new Response(null, { status: 204 }));
    const t = fetchTransport(ORIGIN, fetchFn);
    await t.send({ method: "GET", url: `${ORIGIN}/.ory/sessions/whoami`, headers: [{ name: SESSION_TOKEN_HEADER, value: "tok" }], body: new Uint8Array() });
    expect((calls[0].init.headers as Headers).get(SESSION_TOKEN_HEADER)).toBe("tok");
    expect(calls[0].init.body).toBeUndefined();
    const response = await t.send({ method: "PUT", url: "https://bucket.example/g/1/o/2/full", headers: [], body: Uint8Array.from([1]) });
    expect(calls[1].init.credentials).toBe("omit");
    expect(response.status).toBe(204);
    expect(response.body).toEqual(new Uint8Array());
  });

  it("lets a network failure reject", async () => {
    const t = fetchTransport(ORIGIN, async () => {
      throw new TypeError("Failed to fetch");
    });
    await expect(t.send({ method: "GET", url: `${ORIGIN}/x`, headers: [], body: new Uint8Array() })).rejects.toThrow("Failed to fetch");
  });
});
