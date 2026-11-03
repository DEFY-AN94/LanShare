// Rendering components in tests that run in happy-dom (a test file starts with
// `// @vitest-environment happy-dom`), with React's own act(). Only tests
// import this; the app build never includes it.

import { act, StrictMode, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: Root[] = [];

afterEach(() => {
  act(() => roots.splice(0).forEach((r) => r.unmount()));
  document.body.innerHTML = "";
  window.getSelection()?.removeAllRanges();
  vi.restoreAllMocks();
});

/** Mounts `ui` in StrictMode, as main.tsx does, so effects also run twice here. */
export function render(ui: ReactNode) {
  const host = document.body.appendChild(document.createElement("div"));
  const root = createRoot(host);
  roots.push(root);
  act(() => root.render(<StrictMode>{ui}</StrictMode>));
  return { rerender: (next: ReactNode) => act(() => root.render(<StrictMode>{next}</StrictMode>)) };
}

/** What `(pointer: coarse)` answers: the iPhone, or the computer's mouse. */
export function pointer(kind: "coarse" | "fine") {
  vi.spyOn(window, "matchMedia").mockImplementation((query: string) => ({ matches: query === `(pointer: ${kind})`, media: query }) as MediaQueryList);
}

export const byText = (text: string, selector = "button") =>
  Array.from(document.querySelectorAll<HTMLElement>(selector)).find((el) => el.textContent?.trim() === text) ?? null;

export const byLabel = (label: string) => document.querySelector<HTMLElement>(`[aria-label="${label}"]`);

/** A key pressed where focus is, as the browser sends it (it bubbles to window). */
export function press(key: string, init: KeyboardEventInit & { keyCode?: number } = {}) {
  const target = document.activeElement ?? document.body;
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}

export function click(el: Element | null) {
  if (!el) throw new Error("nothing to click");
  act(() => (el as HTMLElement).click());
}

export function fire(el: Element | null, event: Event) {
  if (!el) throw new Error("no target");
  act(() => {
    el.dispatchEvent(event);
  });
  return event;
}

/** Lets awaited promises (a dialog's answer) settle. */
export const flush = () => act(async () => {});
