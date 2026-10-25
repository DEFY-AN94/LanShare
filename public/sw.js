// Offline shell for the phone. Keeps a copy of the app's page and its
// scripts, so the Home Screen app still opens when the computer cannot be
// reached (no Wi-Fi, the computer asleep) and shows its own "can't reach the
// computer" screen, which keeps retrying by itself. Without it, iOS shows a
// bare "Safari cannot open the page" error.
//
// Only the page and /assets/ are stored. Nothing from /api/ (messages,
// files, the event stream) ever goes through this cache, and neither does
// pairing or installing the certificate (/pair) or a page opened with
// ?direct: the app's way past the stored copy, for when the computer's
// certificate changed.

const CACHE = "lanshare-shell-v1";
const SHELL = "/";
/** On a working LAN the page loads in well under a second. */
const NAV_TIMEOUT_MS = 3000;

self.addEventListener("install", (event) => {
  self.skipWaiting();
  // The page that registered this worker loaded without it, and a Home Screen
  // app has storage of its own: store a copy now, so the very next start
  // works offline. Best effort.
  event.waitUntil(precache().catch(() => undefined));
});
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin || url.pathname.startsWith("/api/")) return;
  if (req.mode === "navigate") {
    if (url.pathname !== "/pair" && !url.searchParams.has("direct")) event.respondWith(page(event));
  } else if (url.pathname.startsWith("/assets/")) event.respondWith(asset(req));
});

const isPage = (res) => res.ok && (res.headers.get("content-type") || "").includes("text/html");
const assetsIn = (html) => [...new Set(html.match(/\/assets\/[^"'\s)]+/g) || [])];

async function precache() {
  const res = await fetch(SHELL, { cache: "no-store" });
  if (!isPage(res)) return;
  const html = await res.clone().text();
  const cache = await caches.open(CACHE);
  // The built files first: a stored page is only useful with them.
  await cache.addAll(assetsIn(html));
  await cache.put(SHELL, res);
}

/** The network first, so a reachable computer always serves the current version. */
async function page(event) {
  const network = fetch(event.request).then((res) => {
    if (isPage(res)) keep(event, res.clone());
    return res;
  });
  try {
    return await Promise.race([network, new Promise((_, reject) => setTimeout(() => reject(new Error("timeout")), NAV_TIMEOUT_MS))]);
  } catch {
    const cached = await caches
      .open(CACHE)
      .then((c) => c.match(SHELL))
      .catch(() => undefined);
    // Nothing stored yet: wait for the network's own answer or error.
    return cached || network;
  }
}

/** Stores a fresh page in the background; a failure here never affects the page itself. */
function keep(event, res) {
  const saving = remember(res).catch(() => undefined);
  try {
    event.waitUntil(saving);
  } catch {
    // The answer already went out (from the stored copy); saving goes on while the worker runs.
  }
}

/** Stores a newer page, but only once its built files are stored too: a page without them opens blank. */
async function remember(res) {
  const html = await res.clone().text();
  const cache = await caches.open(CACHE);
  const missing = [];
  for (const url of assetsIn(html)) if (!(await cache.match(url))) missing.push(url);
  // Throws if one cannot be fetched: the page stored before stays, with its files.
  await cache.addAll(missing);
  await cache.put(SHELL, res);
  await prune(cache, html);
}

/** Built files have content hashes in their names, so a stored copy never goes stale. */
async function asset(req) {
  const cache = await caches.open(CACHE).catch(() => undefined);
  const cached = await cache?.match(req).catch(() => undefined);
  if (cached) return cached;
  const res = await fetch(req);
  if (res.ok && cache) await cache.put(req, res.clone()).catch(() => undefined);
  return res;
}

/** Drops built files the current page no longer uses. */
async function prune(cache, html) {
  const used = new Set(assetsIn(html));
  for (const req of await cache.keys()) {
    const path = new URL(req.url).pathname;
    if (path.startsWith("/assets/") && !used.has(path)) await cache.delete(req);
  }
}
