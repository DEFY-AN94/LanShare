// The phone's offline shell (public/sw.js), run against stand-ins for the
// browser's cache and the network.

import { afterEach, describe, expect, it, vi } from "vitest";
import source from "../public/sw.js?raw";

const ORIGIN = "https://192.168.1.5:39210";
type Req = string | { url: string };
type Net = (url: string) => Promise<Response>;

const abs = (r: Req) => new URL(typeof r === "string" ? r : r.url, ORIGIN).href;
const html = '<!doctype html><script type="module" src="/assets/index-a1.js"></script><link rel="stylesheet" href="/assets/index-a1.css">';
const page = (body = html) => new Response(body, { headers: { "content-type": "text/html; charset=utf-8" } });
/** A computer serving one build: its page and the files that page names. */
const build =
  (body: string, files: Record<string, string>): Net =>
  async (url) => {
    const path = new URL(url).pathname;
    if (path === "/") return page(body);
    if (path in files) return new Response(files[path]);
    return new Response("not found", { status: 404 });
  };
const computer = build(html, { "/assets/index-a1.js": "js", "/assets/index-a1.css": "css" });
const unreachable: Net = async () => {
  throw new TypeError("Load failed");
};

/** Loads the worker; `network` is asked at each request, so a test can switch it. */
function boot(network: () => Net, opts: { failWrites?: boolean } = {}) {
  const listeners: Record<string, (event: unknown) => void> = {};
  const stored = new Map<string, Response>();
  const waits: Promise<unknown>[] = [];
  const cache = {
    async put(req: Req, res: Response) {
      if (opts.failWrites) throw new Error("QuotaExceededError");
      stored.set(abs(req), res);
    },
    async match(req: Req) {
      return stored.get(abs(req))?.clone();
    },
    async addAll(urls: string[]) {
      for (const u of urls) {
        const res = await network()(abs(u));
        if (!res.ok) throw new TypeError("bad response");
        await cache.put(u, res);
      }
    },
    async keys() {
      return [...stored.keys()].map((url) => ({ url }));
    },
    async delete(req: Req) {
      return stored.delete(abs(req));
    },
  };
  const self = {
    location: { origin: ORIGIN },
    addEventListener: (type: string, fn: (event: unknown) => void) => void (listeners[type] = fn),
    skipWaiting: () => undefined,
    clients: { claim: async () => undefined },
  };
  const fetch = vi.fn((req: Req) => network()(abs(req)));
  new Function("self", "caches", "fetch", "setTimeout", source)(self, { open: async () => cache }, fetch, setTimeout);
  return {
    fetch,
    storedPaths: () => [...stored.keys()].map((u) => new URL(u).pathname).sort(),
    async install() {
      listeners.install({ waitUntil: (p: Promise<unknown>) => waits.push(p) });
      await Promise.all(waits);
    },
    /** What the worker answers, or undefined when it leaves the request to the network. */
    request(path: string, mode = "navigate"): Promise<Response> | undefined {
      let answer: Promise<Response> | undefined;
      listeners.fetch({
        request: { method: "GET", url: abs(path), mode },
        respondWith: (p: Promise<Response>) => void (answer = p),
        waitUntil: (p: Promise<unknown>) => waits.push(p),
      });
      return answer;
    },
    settle: () => Promise.all(waits),
  };
}

afterEach(() => vi.useRealTimers());

describe("offline shell", () => {
  it("stores the page and its files on install, so the next start works without the computer", async () => {
    let net = computer;
    const sw = boot(() => net);
    await sw.install();
    expect(sw.storedPaths()).toEqual(["/", "/assets/index-a1.css", "/assets/index-a1.js"]);

    net = unreachable;
    expect(await (await sw.request("/"))!.text()).toBe(html);
    expect(await (await sw.request("/assets/index-a1.js", "no-cors"))!.text()).toBe("js");
  });

  it("falls back to the stored page when the computer does not answer within 3 s", async () => {
    vi.useFakeTimers();
    let net = computer;
    const sw = boot(() => net);
    await sw.install();
    net = () => new Promise(() => undefined);
    const answer = sw.request("/")!;
    await vi.advanceTimersByTimeAsync(3_000);
    expect(await (await answer).text()).toBe(html);
  });

  it("always serves a reachable computer's page, even when storing it fails", async () => {
    const sw = boot(() => async () => page("<p>current</p>"), { failWrites: true });
    expect(await (await sw.request("/"))!.text()).toBe("<p>current</p>");
    await sw.settle();
  });

  it("stores a newer page only together with its files, and drops the old ones", async () => {
    let net = computer;
    const sw = boot(() => net);
    await sw.install();
    const v2 = '<!doctype html><script type="module" src="/assets/index-b2.js"></script>';
    net = build(v2, { "/assets/index-b2.js": "js 2" });
    expect(await (await sw.request("/"))!.text()).toBe(v2);
    await sw.settle();
    expect(sw.storedPaths()).toEqual(["/", "/assets/index-b2.js"]);

    // A build whose files cannot be fetched: the page stored before stays, with its files.
    const v3 = '<!doctype html><script type="module" src="/assets/index-c3.js"></script>';
    net = build(v3, {});
    await sw.request("/");
    await sw.settle();
    net = unreachable;
    expect(await (await sw.request("/"))!.text()).toBe(v2);
    expect(sw.storedPaths()).toEqual(["/", "/assets/index-b2.js"]);
  });

  it("leaves pairing, the certificate page, ?direct and the API to the network", () => {
    const sw = boot(() => unreachable);
    expect(sw.request("/pair?t=one-time")).toBeUndefined();
    expect(sw.request("/pair?cert")).toBeUndefined();
    expect(sw.request("/?direct=1")).toBeUndefined();
    expect(sw.request("/api/me", "cors")).toBeUndefined();
    expect(sw.fetch).not.toHaveBeenCalled();
  });
});
