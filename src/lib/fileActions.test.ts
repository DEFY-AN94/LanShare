// Where a phone opens a file: the Home Screen app, or a Safari tab.

import { afterEach, expect, it, vi } from "vitest";
import { inlineUrl, isStandalone } from "./fileActions";

afterEach(() => vi.unstubAllGlobals());

function stub(displayMode: string, standalone?: boolean) {
  vi.stubGlobal("window", { matchMedia: (q: string) => ({ matches: q === `(display-mode: ${displayMode})` }) });
  vi.stubGlobal("navigator", { standalone });
}

it("tells the Home Screen app from a Safari tab", () => {
  stub("standalone");
  expect(isStandalone()).toBe(true);
  stub("browser", true);
  expect(isStandalone()).toBe(true);
  stub("browser", false);
  expect(isStandalone()).toBe(false);
  stub("browser");
  expect(isStandalone()).toBe(false);
});

it("adds inline=1 to either kind of file URL", () => {
  expect(inlineUrl("/api/messages/7/file")).toBe("/api/messages/7/file?inline=1");
  expect(inlineUrl("/api/files/download?path=a%20b.mp4")).toBe("/api/files/download?path=a%20b.mp4&inline=1");
});
