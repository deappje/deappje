// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { Shell } from "./App";
import type { Client } from "./core";
import { addMember, invite, newGroup, ofType, post, profile, syncAll, World } from "./core/testing";
import { Approvals } from "./screens/Approvals";
import { Groups } from "./screens/Groups";
import { SignIn } from "./screens/SignIn";
import { Timeline } from "./screens/Timeline";
import { type SyncState, useSync } from "./sync";

// Feature suite, "Screens" (docs/01-deappje/03-testing.md): the React
// screens under jsdom, each over a real client on the wasm ModelServer,
// inside the app's Shell with the app's own sync hook.

let world: World;
beforeAll(async () => {
  world = await World.open(16);
});
afterAll(() => world?.free());
afterEach(cleanup);

/** A screen as the app frames it: the Shell, the sync hook, the screen drawn from its state. */
function Screen({ client, children }: { client: Client; children: (sync: SyncState) => ReactNode }) {
  const sync = useSync(client, true, 60_000);
  return (
    <Shell signedIn onLink={() => {}} syncing={sync.syncing} syncError={sync.error} offline={sync.offline}>
      {children(sync)}
    </Shell>
  );
}

/** The browser came to the front: the shell's wake-up. */
function focus() {
  act(() => {
    window.dispatchEvent(new Event("focus"));
  });
}

const indicator = () => document.querySelector("span.sync")!;

describe("screens", () => {
  it("F60 sign-in shows the one button and, signed in, the groups list", async () => {
    const signedOut = render(
      <Shell signedIn={false} onLink={() => {}}>
        <SignIn remember={false} onRemember={() => {}} returnTo="https://app.example/" joining={false} />
      </Shell>,
    );
    const button = screen.getByRole("link", { name: "Sign in with Basable" });
    expect(button.getAttribute("href")).toBe("/.ory/self-service/login/browser?return_to=https%3A%2F%2Fapp.example%2F");
    expect(signedOut.container.querySelectorAll("a.button, button")).toHaveLength(1);
    expect(screen.queryByRole("link", { name: "Groups" })).toBeNull();
    expect(screen.getByRole("checkbox")).toHaveProperty("checked", false);
    cleanup();

    const { A } = world.users();
    const a = await world.browser(A);
    const g1 = await newGroup(a, "Family");
    const g2 = await newGroup(a, "Friends");
    render(<Screen client={a.client}>{(sync) => <Groups client={a.client} sync={sync} />}</Screen>);
    expect((await screen.findByRole("link", { name: "Family" })).getAttribute("href")).toBe(`/g/${g1}`);
    expect(screen.getByRole("link", { name: "Friends" }).getAttribute("href")).toBe(`/g/${g2}`);
    expect(screen.getByRole("link", { name: "Groups" })).toBeDefined();
    expect(screen.getByRole("link", { name: "Approvals" })).toBeDefined();
    expect(screen.queryByText("Sign in with Basable")).toBeNull();
    a.dispose();
  });

  it("F61 groups: create shows the new group; join by link shows the group without keys and the waiting state", async () => {
    const { A, B } = world.users();
    const a = await world.browser(A);
    const b = await world.browser(B);
    render(<Screen client={a.client}>{(sync) => <Groups client={a.client} sync={sync} />}</Screen>);
    await screen.findByText("No groups yet. Create one, or paste an invite link.");
    fireEvent.change(screen.getByPlaceholderText("A new group's name"), { target: { value: "Hiking" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await screen.findByRole("link", { name: "Hiking" });
    const groups = await a.core.listGroups();
    expect(groups).toHaveLength(1);
    const g = groups[0].id;
    expect(groups[0].holds_keys).toBe(true);
    expect((await a.core.readObject(g, a.core.groupProfileId(g)))?.body).toEqual(profile("Hiking"));
    expect(screen.getByRole("link", { name: "Hiking" }).getAttribute("href")).toBe(`/g/${g}`);
    await waitFor(() => expect(screen.queryByText("new")).toBeNull());
    expect(screen.getByPlaceholderText("A new group's name")).toHaveProperty("value", "");
    cleanup();

    const link = await invite(a, g);
    render(<Screen client={b.client}>{(sync) => <Groups client={b.client} sync={sync} />}</Screen>);
    await screen.findByText("No groups yet. Create one, or paste an invite link.");
    fireEvent.change(screen.getByPlaceholderText("Paste an invite link to join"), { target: { value: link } });
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    await screen.findByText("Waiting for a member to approve this browser");
    expect(screen.getByText("no key yet")).toBeDefined();
    expect(screen.getByRole("link", { name: /Waiting for a member/ }).getAttribute("href")).toBe(`/g/${g}`);
    expect((await b.core.listGroups()).find((x) => x.id === g)).toEqual({ id: g, holds_keys: false, phase: "new" });
    expect((await a.core.pendingKeyRequests()).filter((r) => r.group_id === g)).toHaveLength(1);
    a.dispose();
    b.dispose();
  });

  it("F62 approvals: a pending request shows the fingerprint and whether the invite vouches for it; approving removes it", async () => {
    const { A, B } = world.users();
    const a = await world.browser(A);
    const b = await world.browser(B);
    const g = await newGroup(a, "Us");
    const link = await invite(a, g);
    await b.core.join(link);
    render(<Screen client={a.client}>{(sync) => <Approvals client={a.client} sync={sync} />}</Screen>);
    const fingerprint = await b.core.fingerprint();
    const row = (await screen.findByText(fingerprint)).closest("li")!;
    expect(within(row).getByText("web")).toBeDefined();
    expect(within(row).getByText(new RegExp(`device of ${B}`))).toBeDefined();
    expect(within(row).getByText("The invite link vouches for this device.")).toBeDefined();

    fireEvent.click(within(row).getByRole("button", { name: "Approve" }));
    await screen.findByText("No device is waiting for keys.");
    expect(screen.queryByText(fingerprint)).toBeNull();
    expect((await a.core.pendingKeyRequests()).filter((r) => r.group_id === g)).toEqual([]);
    expect((await b.core.sync()).keys_received).toEqual([g]);

    // A device of one's own came through no invite: the fingerprint decides.
    const a2 = await world.browser(A);
    await a2.core.sync();
    focus();
    const own = (await screen.findByText(await a2.core.fingerprint())).closest("li")!;
    expect(within(own).getByText("No invite: compare the fingerprint with what the device shows.")).toBeDefined();
    expect(within(own).getByText(new RegExp(`device of ${A}`))).toBeDefined();
    fireEvent.click(within(own).getByRole("button", { name: "Approve" }));
    await screen.findByText("No device is waiting for keys.");
    expect((await a2.core.sync()).keys_received).toEqual([g]);
    a.dispose();
    b.dispose();
    a2.dispose();
  });

  it("F63 timeline: posts newest first; compose adds one; edit and delete on own posts only", async () => {
    const { A, B } = world.users();
    const a = await world.browser(A);
    const b = await world.browser(B);
    const g = await newGroup(a, "Us");
    await addMember(a, b, g);
    const first = await post(a, g, "first, by A");
    await syncAll(a, b);
    await post(b, g, "second, by B");
    await syncAll(b, a);

    render(<Screen client={a.client}>{(sync) => <Timeline client={a.client} groupId={g} userId={A} sync={sync} />}</Screen>);
    await screen.findByRole("heading", { name: "Us" });
    const texts = () => screen.getAllByRole("listitem").map((li) => li.querySelector("p.text")?.textContent);
    await waitFor(() => expect(texts()).toEqual(["second, by B", "first, by A"]));
    const item = (text: string) => screen.getByText(text).closest("li")!;
    expect(within(item("first, by A")).getByRole("button", { name: "Edit" })).toBeDefined();
    expect(within(item("first, by A")).getByRole("button", { name: "Delete" })).toBeDefined();
    expect(within(item("second, by B")).queryByRole("button", { name: "Edit" })).toBeNull();
    expect(within(item("second, by B")).queryByRole("button", { name: "Delete" })).toBeNull();

    // Compose: the post is at the top, pending until the screen's sync lands it.
    fireEvent.change(screen.getByPlaceholderText("Write a post"), { target: { value: "third, by A" } });
    fireEvent.click(screen.getByRole("button", { name: "Post" }));
    await waitFor(() => expect(texts()).toEqual(["third, by A", "second, by B", "first, by A"]));
    await waitFor(() => expect(screen.queryByText(/sending/)).toBeNull());
    expect(await ofType(a, g, "Post")).toHaveLength(3);
    expect(screen.getByPlaceholderText("Write a post")).toHaveProperty("value", "");
    await b.core.sync();
    expect((await ofType(b, g, "Post")).map((o) => (o.body.fields as { text: string }).text).sort()).toEqual(["first, by A", "second, by B", "third, by A"]);

    // Edit one's own.
    fireEvent.click(within(item("first, by A")).getByRole("button", { name: "Edit" }));
    const editor = screen.getAllByRole("textbox").find((t) => (t as HTMLTextAreaElement).value === "first, by A")!;
    fireEvent.change(editor, { target: { value: "first, by A, edited" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByText("first, by A, edited");
    expect(within(item("first, by A, edited")).getByText(/\(edited\)/)).toBeDefined();
    await waitFor(() => expect(screen.queryByText(/sending/)).toBeNull());
    await b.core.sync();
    expect((await b.core.readObject(g, first))?.body).toEqual({ type: "Post", fields: { text: "first, by A, edited" } });

    // Delete one's own.
    fireEvent.click(within(item("third, by A")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(texts()).toEqual(["second, by B", "first, by A, edited"]));
    expect(await ofType(a, g, "Post")).toHaveLength(2);
    await waitFor(() => expect(indicator().textContent).toBe(""));
    await b.core.sync();
    expect(await ofType(b, g, "Post")).toHaveLength(2);
    a.dispose();
    b.dispose();
  });

  it("F64 sync indicator: an outage shows the offline state and clears when the server is back", async () => {
    const { A } = world.users();
    const a = await world.browser(A);
    await newGroup(a, "Flaky");
    render(<Screen client={a.client}>{(sync) => <Groups client={a.client} sync={sync} />}</Screen>);
    await screen.findByRole("link", { name: "Flaky" });
    await waitFor(() => expect(indicator().textContent).toBe(""));

    world.server.setOutage(true);
    focus();
    await waitFor(() => expect(indicator().textContent).toBe("offline"));
    expect(indicator().getAttribute("title")).toMatch(/unavailable/);
    // What the browser holds stays on the screen.
    expect(screen.getByRole("link", { name: "Flaky" })).toBeDefined();

    world.server.setOutage(false);
    focus();
    await waitFor(() => expect(indicator().textContent).toBe(""));
    expect(indicator().getAttribute("title")).toBeNull();
    a.dispose();
  });
});
