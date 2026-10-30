import { useEffect, useState } from "react";

// The sign-in pages Kratos sends the browser to (selfservice.flows.*.ui_url
// in k8s/kratos/configmap.yaml): /auth/login, /auth/register,
// /auth/settings and /auth/error. Each draws its Kratos flow from the flow's
// own UI nodes, so every method Kratos offers appears with no code here,
// which today is one: "Sign in with Basable" (single sign-on with every
// app on Basable). A form posts straight to Kratos, which
// answers with the next step of the same flow or with where to go next.
// The bucket answers /auth/... with index.html, so this app renders them.

type UiText = { id: number; text: string; type: string };
type UiNode = {
  type: string;
  group: string;
  attributes: {
    name?: string;
    type?: string;
    value?: string;
    required?: boolean;
    disabled?: boolean;
    autocomplete?: string;
    text?: UiText;
  };
  messages?: UiText[];
  meta?: { label?: UiText };
};
type Flow = { ui: { action: string; method: string; nodes: UiNode[]; messages?: UiText[] } };

const flows: Record<string, string> = { login: "login", register: "registration", settings: "settings" };
const titles: Record<string, string> = { login: "Sign in", register: "Create an account", settings: "Account settings" };

function restart(api: string) {
  window.location.replace(`/.ory/self-service/${api}/browser`);
}

function Notices({ messages }: { messages?: UiText[] }) {
  return (
    <>
      {(messages ?? []).map((m) => (
        <p key={m.id} className={`notice notice-${m.type}`}>
          {m.text}
        </p>
      ))}
    </>
  );
}

// One node of the flow: hidden (the CSRF token, the flow's state), a field,
// or a submit button (a method, or a provider such as Basable).
function Node({ node }: { node: UiNode }) {
  const a = node.attributes;
  if (node.type === "text") return <p>{a.text?.text}</p>;
  if (node.type !== "input") return null;
  const label = node.meta?.label?.text ?? a.name;
  if (a.type === "hidden") return <input type="hidden" name={a.name} value={a.value ?? ""} />;
  if (a.type === "submit" || a.type === "button") {
    return (
      <button type="submit" name={a.name} value={a.value} disabled={a.disabled}>
        {label}
      </button>
    );
  }
  return (
    <label>
      {label}
      <input
        type={a.type}
        name={a.name}
        defaultValue={a.value ?? ""}
        required={a.required}
        disabled={a.disabled}
        autoComplete={a.autocomplete}
      />
      <Notices messages={node.messages} />
    </label>
  );
}

export function Auth() {
  const kind = window.location.pathname.split("/")[2] || "login";
  const params = new URLSearchParams(window.location.search);
  const [flow, setFlow] = useState<Flow | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    if (kind === "error") {
      fetch(`/.ory/self-service/errors?id=${encodeURIComponent(params.get("id") ?? "")}`, { credentials: "include" })
        .then((res) => res.json())
        .then((body) => setFailure(body.error?.reason ?? body.error?.message ?? "Something went wrong."))
        .catch(() => setFailure("Something went wrong."));
      return;
    }
    const api = flows[kind];
    if (!api) {
      window.location.replace("/");
      return;
    }
    const id = params.get("flow");
    if (!id) {
      restart(api);
      return;
    }
    fetch(`/.ory/self-service/${api}/flows?id=${encodeURIComponent(id)}`, {
      credentials: "include",
      headers: { Accept: "application/json" },
    })
      .then((res) => {
        // An expired, foreign or unknown flow starts over.
        if (res.status === 403 || res.status === 404 || res.status === 410) {
          restart(api);
          return null;
        }
        return res.json();
      })
      .then((f) => f && setFlow(f))
      .catch(() => setFailure("Kratos is not reachable yet."));
    // The page is rendered once per navigation: Kratos moves it by redirect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (kind === "error" || (failure && !flow)) {
    return (
      <>
        <h1>{kind === "error" ? "Sign-in failed" : titles[kind]}</h1>
        <p>{failure}</p>
        <a href="/.ory/self-service/login/browser">Try again</a>
      </>
    );
  }
  if (!flow) return <p>Loading…</p>;
  return (
    <>
      <h1>{titles[kind]}</h1>
      <Notices messages={flow.ui.messages} />
      <form action={flow.ui.action} method={flow.ui.method} className="auth">
        {flow.ui.nodes.map((node, i) => (
          <Node key={`${node.attributes.name ?? node.type}-${i}`} node={node} />
        ))}
      </form>
      {kind === "login" && <a href="/.ory/self-service/registration/browser">Create an account</a>}
      {kind === "register" && <a href="/.ory/self-service/login/browser">Sign in instead</a>}
      {kind === "settings" && <a href="/">Back</a>}
    </>
  );
}
