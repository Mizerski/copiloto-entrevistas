import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// Fora do Tauri (navegador comum, em desenvolvimento), simula o backend.
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  const { installDevMock } = await import("./devMock");
  installDevMock();
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
