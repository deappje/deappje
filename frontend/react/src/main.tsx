import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { loadCore } from "./core";
import wasmUrl from "./wasm/deappje_core_bg.wasm?url";
import "./app.css";

// The core first: everything on the page is a view of it.
const root = createRoot(document.getElementById("root")!);
loadCore(wasmUrl).then(
  (module) =>
    root.render(
      <StrictMode>
        <App module={module} />
      </StrictMode>,
    ),
  (e: unknown) => root.render(<p className="panel notice-error">The core did not load: {e instanceof Error ? e.message : String(e)}</p>),
);
