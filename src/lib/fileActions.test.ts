// Where a phone opens a file: the Home Screen app, or a Safari tab.

import { afterEach, expect, it, vi } from "vitest";
import { fetchAsFile, fetchTextStart, inlineUrl, isStandalone } from "./fileActions";

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

it("says a dropped connection in Chinese, and leaves a cancel alone", async () => {
  vi.stubGlobal("fetch", () => Promise.reject(new TypeError("Load failed")));
  await expect(fetchAsFile("/f", "a.zip", "")).rejects.toThrow("连接中断");
  await expect(fetchTextStart("/f")).rejects.toThrow("连接中断");
  const abort = new DOMException("The operation was aborted.", "AbortError");
  vi.stubGlobal("fetch", () => Promise.reject(abort));
  await expect(fetchAsFile("/f", "a.zip", "")).rejects.toBe(abort);
});

it("passes on the server's reason for a refused download", async () => {
  vi.stubGlobal("fetch", () => Promise.resolve(Response.json({ error: "电脑端已暂停共享" }, { status: 503 })));
  await expect(fetchAsFile("/f", "a.zip", "")).rejects.toThrow("下载失败：电脑端已暂停共享");
});

it("reads only the start of a text file, and nothing of an empty one", async () => {
  const seen: string[] = [];
  vi.stubGlobal("fetch", (_url: string, init: RequestInit) => {
    seen.push(new Headers(init.headers).get("Range") ?? "");
    return Promise.resolve(new Response(new Uint8Array([104, 105]), { status: 206 }));
  });
  expect(await fetchTextStart("/f")).toEqual(new Uint8Array([104, 105]));
  expect(seen).toEqual(["bytes=0-1048575"]);
  vi.stubGlobal("fetch", () => Promise.resolve(new Response(null, { status: 416 })));
  expect((await fetchTextStart("/f")).length).toBe(0);
});
