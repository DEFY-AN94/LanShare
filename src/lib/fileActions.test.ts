// Where a phone opens a file (the Home Screen app, or a Safari tab), and what
// the computer's window says when it only shows one in its folder.

import { afterEach, expect, it, vi } from "vitest";
import { fetchAsFile, fetchTextStart, hostOpen, hostOpenMessageFile, inlineUrl, isStandalone } from "./fileActions";

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

it("gives a saved file the type the computer serves unless one is asked for", async () => {
  vi.stubGlobal("fetch", () => Promise.resolve(new Response("let x = 1;\n", { headers: { "Content-Type": "text/plain; charset=utf-8" } })));
  expect((await fetchAsFile("/f", "notes.ts", "")).type).toMatch(/^text\/plain/);
  expect((await fetchAsFile("/f", "clip.mp4", "video/mp4")).type).toBe("video/mp4");
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

it("says why a file was only shown in its folder, and nothing when it opened", async () => {
  vi.stubGlobal("window", { setTimeout, clearTimeout });
  const asked: string[] = [];
  let action = "revealed";
  vi.stubGlobal("fetch", (url: string) => {
    asked.push(url);
    return Promise.resolve(Response.json({ action }));
  });
  const notify = vi.fn();
  await hostOpen("收件箱/model.blend", notify);
  await hostOpenMessageFile(7, notify);
  expect(asked).toEqual(["/api/host/open", "/api/host/messages/7/open"]);
  expect(notify.mock.calls).toEqual([
    ["为了安全，只有常见的图片、视频、音频、PDF、文本、文档和压缩包会直接打开，这个文件已在文件夹中显示"],
    ["为了安全，只有常见的图片、视频、音频、PDF、文本、文档和压缩包会直接打开，这个文件已在文件夹中显示"],
  ]);
  action = "opened";
  await hostOpen("收件箱/photo.jpg", notify);
  await hostOpenMessageFile(8, notify);
  expect(notify).toHaveBeenCalledTimes(2);
});
