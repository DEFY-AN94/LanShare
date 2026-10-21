import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { DialogProvider } from "./components/Dialog";
import { ToastProvider } from "./components/Toast";
import { installViewportFix } from "./lib/viewport";
import "./styles.css";

installViewportFix();

// Phone only (the desktop window loads over http://127.0.0.1): keeps the page
// itself available offline, see public/sw.js. This also works when the
// certificate warning was only tapped through, so a stored page can hide a
// certificate problem; the "can't reach the computer" screen has a way past it.
if ("serviceWorker" in navigator && window.location.protocol === "https:") {
  navigator.serviceWorker.register("/sw.js").catch(() => undefined);
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ToastProvider>
      <DialogProvider>
        <App />
      </DialogProvider>
    </ToastProvider>
  </StrictMode>,
);
