import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { DesktopUpdateProvider } from "./DesktopUpdates";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <DesktopUpdateProvider>
      <App />
    </DesktopUpdateProvider>
  </React.StrictMode>,
);
