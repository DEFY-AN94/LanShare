import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { DialogProvider } from "./components/Dialog";
import { ToastProvider } from "./components/Toast";
import { installViewportFix } from "./lib/viewport";
import "./styles.css";

installViewportFix();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ToastProvider>
      <DialogProvider>
        <App />
      </DialogProvider>
    </ToastProvider>
  </StrictMode>,
);
