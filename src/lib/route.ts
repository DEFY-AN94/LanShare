// Tiny hash router: `#/files/Photos/2025` → ["files", "Photos", "2025"].
// Hash routing keeps Safari's swipe-back working without server routes.

import { useEffect, useState } from "react";

export function parseHash(): string[] {
  const h = window.location.hash.replace(/^#\/?/, "");
  if (!h) return [];
  return h.split("/").map((s) => {
    try {
      return decodeURIComponent(s);
    } catch {
      return s;
    }
  });
}

function toHash(segments: string[]): string {
  return "#/" + segments.filter((s) => s !== "").map(encodeURIComponent).join("/");
}

export function navigate(...segments: string[]) {
  const h = toHash(segments);
  if (window.location.hash !== h) window.location.hash = h;
}

export function replaceRoute(...segments: string[]) {
  const h = toHash(segments);
  if (window.location.hash === h) return;
  history.replaceState(null, "", h);
  window.dispatchEvent(new HashChangeEvent("hashchange"));
}

export function useHashRoute(): string[] {
  const [route, setRoute] = useState(parseHash);
  useEffect(() => {
    const onChange = () => setRoute(parseHash());
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return route;
}
