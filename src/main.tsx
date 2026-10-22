import React from "react";
import ReactDOM from "react-dom/client";
import "./ui/theme.css";

// Each window loads index.html with ?view=… (see src-tauri/src/windows.rs).
const params = new URLSearchParams(location.search);
const view = params.get("view");

async function start() {
  if (import.meta.env.DEV && params.has("mock")) await import("./mock");
  const { default: Root } =
    view === "companion" ? await import("./companion/Companion") : view === "preview" ? await import("./preview") : await import("./panel/Panel");
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <Root />
    </React.StrictMode>,
  );
}
start();
