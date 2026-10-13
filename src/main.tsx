import React from "react";
import ReactDOM from "react-dom/client";
import "@radix-ui/themes/styles.css";
import { Theme } from "@radix-ui/themes";
import App from "./App";
import { useSystemDark } from "./ui";

function Root() {
  const dark = useSystemDark();
  return (
    <Theme
      appearance={dark ? "dark" : "light"}
      accentColor="orange"
      grayColor="sand"
      radius="medium"
      scaling="95%"
      panelBackground="solid"
    >
      <App />
    </Theme>
  );
}

async function main() {
  // Fora do Tauri (navegador comum, em desenvolvimento), simula o backend.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { installDevMock } = await import("./devMock");
    installDevMock();
  }
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <Root />
    </React.StrictMode>,
  );
}

main();
