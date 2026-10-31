import { readFile } from "node:fs/promises";
import { expect } from "vitest";
import type { Core, ModelServer, ObjectBody, ObjectView, SecretsSeam } from "../wasm/deappje_core.js";
import { type Client, clientOver } from "./client";
import { type CoreModule, loadCore } from "./load";
import { softwareDeviceKey } from "./seams/deviceKey";
import { webRandom } from "./seams/random";
import { memorySecrets } from "./seams/secrets";
import { memoryStorage } from "./seams/storage";

/** The core for a test under Node: the .wasm read from beside its glue. */
export function loadCoreForTests(): Promise<CoreModule> {
  return loadCore(readFile(new URL("../wasm/deappje_core_bg.wasm", import.meta.url)).then((bytes) => new Uint8Array(bytes)));
}

/** Bytes from hex. */
export function hex(s: string): Uint8Array {
  return Uint8Array.from(s.match(/../g)!.map((b) => parseInt(b, 16)));
}

/** Hex from bytes. */
export function toHex(bytes: Uint8Array): string {
  return [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
}

// ---- The feature suite's world (docs/01-deappje/03-testing.md, "Feature suites") ----

export const ORIGIN = "https://app.example";
export const USER_A = "00000000-0000-0000-0000-00000000000a";
export const USER_B = "00000000-0000-0000-0000-00000000000b";
export const USER_C = "00000000-0000-0000-0000-00000000000c";

/** The relay's window and the restore grace (`02-relay.md`). */
export const HOUR_MS = 60 * 60 * 1000;
export const WINDOW_MS = 672 * HOUR_MS;
export const RESTORE_GRACE_MS = 30 * 60 * 1000;

/** A clock a test moves. It ticks one millisecond per reading, so two writes never share a time. */
export interface TestClock {
  nowMs(): number;
  advance(ms: number): void;
}

export function testClock(start: number): TestClock {
  let t = start;
  return {
    nowMs: () => (t += 1),
    advance: (ms) => {
      t += ms;
    },
  };
}

/** What a browser keeps: reopening over it is the same device (F03). */
export interface Kept {
  storage: ReturnType<typeof memoryStorage>;
  secrets: SecretsSeam;
  deviceSecret: Uint8Array;
}

/** One browser of the suite: the shell's client over the model server, its seams in view. */
export interface Browser extends Kept {
  client: Client;
  core: Core;
  userId: string;
  clock: TestClock;
  dispose(): void;
}

/** One model server and the browsers over it. */
export class World {
  readonly server: ModelServer;
  private n = 0;
  private u = 0;

  constructor(
    readonly module: CoreModule,
    seed = 1,
  ) {
    this.server = new module.ModelServer(ORIGIN, new Uint8Array(32).fill(seed));
  }

  static async open(seed = 1): Promise<World> {
    return new World(await loadCoreForTests(), seed);
  }

  /**
   * A browser signed in as `userId`: the server knows a token, the core is
   * given it (`setSessionToken`), `restoreSession` yields the identity and
   * the device registers as `web`. Over `kept`, it is a browser reopened:
   * the same storage, secrets and key, the session read back from the
   * secrets.
   */
  async browser(userId: string, kept?: Kept): Promise<Browser> {
    const n = ++this.n;
    const { storage, secrets, deviceSecret } = kept ?? { storage: memoryStorage(), secrets: memorySecrets(), deviceSecret: new Uint8Array(32).fill(n) };
    const clock = testClock(1_700_000_000_000 + n * 1_000_000);
    const client = clientOver(this.module, { transport: this.server, storage, secrets, deviceKey: softwareDeviceKey(this.module, deviceSecret), random: webRandom, clock, origin: ORIGIN });
    const core = client.core;
    if (!kept) {
      const token = `tok-${n}`;
      this.server.signIn(token, userId);
      await core.setSessionToken(token);
    }
    const identity = await core.restoreSession();
    expect(identity?.id).toBe(userId);
    await core.registerDevice("web");
    return { client, core, userId, storage, secrets, deviceSecret, clock, dispose: () => client.dispose() };
  }

  /** A browser signed in as nobody: no token. */
  anonymous(): Browser {
    const n = ++this.n;
    const kept: Kept = { storage: memoryStorage(), secrets: memorySecrets(), deviceSecret: new Uint8Array(32).fill(n) };
    const clock = testClock(1_700_000_000_000 + n * 1_000_000);
    const client = clientOver(this.module, { transport: this.server, ...kept, deviceKey: softwareDeviceKey(this.module, kept.deviceSecret), random: webRandom, clock, origin: ORIGIN });
    return { client, core: client.core, userId: "", clock, ...kept, dispose: () => client.dispose() };
  }

  /** Three users nobody else in the world is: A, A's second device signs in as A, B and C are others. */
  users(): { A: string; B: string; C: string } {
    const next = () => `10000000-0000-4000-8000-${(++this.u).toString(16).padStart(12, "0")}`;
    return { A: next(), B: next(), C: next() };
  }

  /** Moves the relay's clock past the window and runs its sweeps. */
  windowPasses(): void {
    this.server.advance(WINDOW_MS + HOUR_MS);
    this.server.sweep();
  }

  free(): void {
    this.server.free();
  }
}

export function profile(name: string, description = ""): ObjectBody {
  return { type: "GroupProfile", fields: { name, description, avatar: [], kind: "Group" } };
}

/** A group with its profile, synced: the creator holds it. */
export async function newGroup(a: Browser, name: string): Promise<string> {
  const g = await a.core.createGroup();
  await a.core.createObject(g, undefined, profile(name));
  const report = await a.core.sync();
  expect(report.failed).toEqual([]);
  return g;
}

/** A post by `a`, in the outbox until its next sync. */
export function post(a: Browser, g: string, text: string): Promise<string> {
  return a.core.createObject(g, undefined, { type: "Post", fields: { text } });
}

/** An invite link as shared: the origin and all. */
export async function invite(a: Browser, g: string, role = "member"): Promise<string> {
  const link = await a.core.createInvite(g, role, 0, 0);
  await a.core.sync();
  return ORIGIN + link;
}

/** Holder `a` approves the request of `device`, found by its device id. */
export async function approve(a: Browser, device: Browser): Promise<void> {
  const id = await device.core.deviceId();
  const pending = await a.core.pendingKeyRequests();
  const request = pending.find((r) => r.device_id === id);
  expect(request, `a request from ${id}`).toBeDefined();
  await a.core.approveKeyRequest(request!.request_id);
}

/** `b` joins `a`'s group through an invite and is approved; afterwards `b` holds the key. */
export async function addMember(a: Browser, b: Browser, g: string, role = "member"): Promise<void> {
  const link = await invite(a, g, role);
  const joined = await b.core.join(link);
  expect(joined.group_id).toBe(g);
  await approve(a, b);
  const report = await b.core.sync();
  expect(report.keys_received).toContain(g);
}

/** `a2`, another device of `a`'s user, asks on its sync and `a` approves it. */
export async function addOwnDevice(a: Browser, a2: Browser, g: string): Promise<void> {
  const asked = await a2.core.sync();
  expect(asked.failed).toEqual([]);
  await approve(a, a2);
  const report = await a2.core.sync();
  expect(report.keys_received).toContain(g);
}

export async function syncAll(...browsers: Browser[]): Promise<void> {
  for (const b of browsers) {
    const report = await b.core.sync();
    expect(report.failed).toEqual([]);
  }
}

/** The fields of an object, typed by the caller. */
export function fields<T>(o: ObjectView | undefined): T {
  expect(o).toBeDefined();
  return o!.body.fields as T;
}

/** The objects of a type, in id order as `listObjects` gives them. */
export async function ofType(b: Browser, g: string, type: ObjectBody["type"]): Promise<ObjectView[]> {
  return (await b.core.listObjects(g)).filter((o) => o.body.type === type);
}

/** The error a promise rejects with. */
export async function failure(p: Promise<unknown>): Promise<Error & { kind?: string; code?: string }> {
  try {
    await p;
  } catch (e) {
    return e as Error & { kind?: string; code?: string };
  }
  throw new Error("the promise resolved");
}
