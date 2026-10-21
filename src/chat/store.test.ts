// The chat store's queueing, retry and merge rules, with the network and the
// event stream replaced by stand-ins.

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Message } from "../lib/types";

const h = vi.hoisted(() => {
  class ApiError extends Error {
    status: number;
    constructor(status: number, message: string) {
      super(message);
      this.status = status;
    }
  }
  const handlers = new Map<string, Set<(data: unknown) => void>>();
  const stateHandlers = new Set<(s: string) => void>();
  const events = {
    state: "offline",
    on(name: string, fn: (data: unknown) => void) {
      if (!handlers.has(name)) handlers.set(name, new Set());
      handlers.get(name)!.add(fn);
      return () => handlers.get(name)!.delete(fn);
    },
    onState(fn: (s: string) => void) {
      stateHandlers.add(fn);
      return () => stateHandlers.delete(fn);
    },
  };
  return {
    ApiError,
    events,
    handlers,
    stateHandlers,
    emit: (name: string, data: unknown) => handlers.get(name)?.forEach((fn) => fn(data)),
    setState: (s: string) => {
      events.state = s;
      stateHandlers.forEach((fn) => fn(s));
    },
    api: { get: vi.fn(), post: vi.fn(), del: vi.fn() },
    uploadFile: vi.fn(),
  };
});

vi.mock("../lib/events", () => ({ events: h.events }));
vi.mock("../lib/wakeLock", () => ({ holdScreen: () => () => undefined }));
vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return { api: h.api, qs: actual.qs, uploadFile: h.uploadFile, ApiError: h.ApiError };
});

// The browser pieces the store touches: localStorage and window events.
const storage = new Map<string, string>();
/** Storage listeners in the order the store instances ("tabs") added them. */
const storageListeners: ((e: { key: string | null; newValue: string | null }) => void)[] = [];
Object.assign(globalThis, {
  window: {
    addEventListener: (type: string, fn: (e: { key: string | null; newValue: string | null }) => void) => {
      if (type === "storage") storageListeners.push(fn);
    },
    localStorage: {
      getItem: (k: string) => storage.get(k) ?? null,
      setItem: (k: string, v: string) => void storage.set(k, v),
      removeItem: (k: string) => void storage.delete(k),
      key: (i: number) => [...storage.keys()][i] ?? null,
      get length() {
        return storage.size;
      },
    },
  },
});

const msg = (id: number, extra: Partial<Message> = {}): Message =>
  ({ id, device_id: "d1", from_host: false, client_id: null, kind: "text", text: `m${id}`, file: null, created_at: id * 10, ...extra }) as Message;

const settle = () => new Promise((r) => setTimeout(r, 0));
const outbox = () =>
  [...storage.entries()].filter(([k]) => k.startsWith("lanshare.outbox.")).map(([, v]) => JSON.parse(v) as { client_id: string; text: string });
const queue = (client_id: string, text: string, created_at = Date.now()) =>
  storage.set(`lanshare.outbox.${client_id}`, JSON.stringify({ client_id, text, created_at }));

async function freshStore() {
  vi.resetModules();
  return import("./store");
}

beforeEach(() => {
  storage.clear();
  storageListeners.length = 0;
  h.handlers.clear();
  h.stateHandlers.clear();
  h.events.state = "offline";
  h.api.get.mockReset().mockResolvedValue({ messages: [], has_more: false });
  h.api.post.mockReset();
  h.api.del.mockReset().mockResolvedValue(undefined);
  h.uploadFile.mockReset();
});

describe("sending", () => {
  it("waits while offline, then sends once with the same client_id", async () => {
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    store.sendText("self", "hi");
    const [p] = store.getConversation("self").pending;
    expect(p.status).toBe("waiting");
    expect(h.api.post).not.toHaveBeenCalled();
    expect(outbox().map((o) => o.client_id)).toEqual([p.client_id]);

    h.api.post.mockResolvedValue(msg(1, { client_id: p.client_id, text: "hi" }));
    h.setState("open");
    await settle();
    expect(h.api.post).toHaveBeenCalledTimes(1);
    expect(h.api.post.mock.calls[0][1]).toMatchObject({ client_id: p.client_id, text: "hi" });
    expect(store.getConversation("self").pending).toEqual([]);
    expect(store.getConversation("self").messages.map((m) => m.id)).toEqual([1]);
    expect(outbox()).toEqual([]);
  });

  it("keeps unsent texts across a reload and sends them with their client_id", async () => {
    queue("c-9", "typed earlier");
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    expect(store.getConversation("self").pending.map((p) => [p.client_id, p.status])).toEqual([["c-9", "waiting"]]);

    h.api.post.mockResolvedValue(msg(5, { client_id: "c-9" }));
    h.setState("open");
    await settle();
    expect(h.api.post.mock.calls.map((c) => c[1].client_id)).toEqual(["c-9"]);
    expect(store.getConversation("self").pending).toEqual([]);
  });

  it("does not retry a refused text, but waits again when the computer is unreachable", async () => {
    h.events.state = "open";
    const store = await freshStore();
    await store.ensureLoaded("self", null);

    h.api.post.mockRejectedValueOnce(new h.ApiError(400, "消息太长"));
    store.sendText("self", "too long");
    await settle();
    expect(store.getConversation("self").pending[0]).toMatchObject({ status: "failed", retryable: false });
    expect(outbox(), "a refused text is not kept for later").toEqual([]);
    h.setState("offline");
    h.setState("open");
    await settle();
    expect(h.api.post).toHaveBeenCalledTimes(1);

    // The connection drops while the text is on its way: it waits, with no quick retry.
    h.api.post.mockImplementationOnce(async () => {
      h.events.state = "offline";
      throw new h.ApiError(0, "连接不上电脑");
    });
    store.sendText("self", "later");
    await settle();
    const waiting = store.getConversation("self").pending[1];
    expect(waiting.status).toBe("waiting");
    expect(waiting.quickRetries ?? 0, "no retry while offline").toBe(0);
  });

  it("keeps the texts that two tabs queued", async () => {
    // Both tabs are open before either queues anything.
    const a = await freshStore();
    await a.ensureLoaded("self", null);
    const b = await freshStore();
    await b.ensureLoaded("self", null);
    a.sendText("self", "from tab A");
    b.sendText("self", "from tab B");
    expect(outbox().map((o) => o.text).sort()).toEqual(["from tab A", "from tab B"]);
    // Tab B drops its own text; tab A's stays stored.
    b.discard("self", b.getConversation("self").pending.find((p) => p.text === "from tab B")!.client_id);
    expect(outbox().map((o) => o.text)).toEqual(["from tab A"]);
  });

  it("does not send a text queued more than 30 days ago by itself", async () => {
    queue("c-old", "from last month", Date.now() - 31 * 86_400_000);
    queue("c-new", "from today");
    h.events.state = "open";
    h.api.post.mockResolvedValue(msg(4, { client_id: "c-new" }));
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    await settle();
    expect(h.api.post.mock.calls.map((c) => c[1].client_id)).toEqual(["c-new"]);
    expect(store.getConversation("self").pending).toMatchObject([{ client_id: "c-old", status: "failed", expired: true }]);
    expect(outbox().map((o) => o.client_id), "kept until the user acts on it").toEqual(["c-old"]);
    h.api.post.mockResolvedValue(msg(5, { client_id: "c-old" }));
    store.retry("self", "c-old");
    await settle();
    expect(h.api.post.mock.calls.map((c) => c[1].client_id)).toEqual(["c-new", "c-old"]);
    expect(outbox()).toEqual([]);
  });

  it("a text left waiting for 30 days on an open page stays stored, but waits for 重试", async () => {
    vi.useFakeTimers();
    try {
      const store = await freshStore();
      await store.ensureLoaded("self", null);
      store.sendText("self", "before the trip");
      vi.setSystemTime(Date.now() + 31 * 86_400_000);
      h.setState("open");
      await vi.advanceTimersByTimeAsync(0);
      expect(h.api.post).not.toHaveBeenCalled();
      expect(store.getConversation("self").pending[0]).toMatchObject({ status: "failed", expired: true });
      expect(outbox()).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("sends an expired text the user tapped, even if it has to wait first, also after a reload", async () => {
    queue("c-old", "from last month", Date.now() - 31 * 86_400_000);
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    store.retry("self", "c-old");
    expect(store.getConversation("self").pending[0]).toMatchObject({ status: "waiting", confirmed: true });
    // iOS reloads the page before the computer is back.
    const again = await freshStore();
    await again.ensureLoaded("self", null);
    expect(again.getConversation("self").pending[0]).toMatchObject({ client_id: "c-old", status: "waiting" });
    expect(outbox()).toMatchObject([{ client_id: "c-old", confirmed: true }]);
    h.api.post.mockResolvedValue(msg(6, { client_id: "c-old" }));
    h.setState("open");
    await settle();
    // The first page is still alive here too, so both send it, with one client id: the computer keeps one copy.
    expect(new Set(h.api.post.mock.calls.map((c) => c[1].client_id))).toEqual(new Set(["c-old"]));
    expect(again.getConversation("self").pending).toEqual([]);
  });

  it("does not send a failed text that turned 30 days old by itself either", async () => {
    vi.useFakeTimers();
    try {
      h.events.state = "open";
      const store = await freshStore();
      await store.ensureLoaded("self", null);
      h.api.post.mockRejectedValueOnce(new h.ApiError(503, "电脑端已暂停共享"));
      store.sendText("self", "sent as sharing paused");
      await vi.advanceTimersByTimeAsync(0);
      expect(store.getConversation("self").pending[0]).toMatchObject({ status: "failed", retryable: true });
      vi.setSystemTime(Date.now() + 31 * 86_400_000);
      h.setState("offline");
      h.setState("open");
      await vi.advanceTimersByTimeAsync(0);
      expect(h.api.post).toHaveBeenCalledTimes(1);
      expect(store.getConversation("self").pending[0]).toMatchObject({ status: "failed", expired: true });
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not send a text that another tab deleted, whether or not it heard about it", async () => {
    const a = await freshStore();
    await a.ensureLoaded("self", null);
    const tabA = storageListeners.length;
    a.sendText("self", "deleted in tab B");
    a.sendText("self", "deleted in tab B too");
    const [first, second] = a.getConversation("self").pending.map((p) => p.client_id);
    const b = await freshStore();
    await b.ensureLoaded("self", null);
    b.discard("self", first);
    b.discard("self", second);
    // Tab A hears about the first one; it slept through the second.
    storageListeners.slice(0, tabA).forEach((fn) => fn({ key: `lanshare.outbox.${first}`, newValue: null }));
    expect(a.getConversation("self").pending.map((p) => p.client_id)).toEqual([second]);
    h.setState("open");
    await settle();
    expect(h.api.post).not.toHaveBeenCalled();
    expect(a.getConversation("self").pending).toEqual([]);
  });

  it("offers 重试 once the quick retries run out while the connection stays up", async () => {
    vi.useFakeTimers();
    try {
      h.events.state = "open";
      const store = await freshStore();
      await store.ensureLoaded("self", null);
      h.api.post.mockRejectedValue(new h.ApiError(0, "连接不上电脑"));
      store.sendText("self", "slow computer");
      for (let i = 0; i < 3; i++) await vi.advanceTimersByTimeAsync(2_000);
      expect(h.api.post).toHaveBeenCalledTimes(4);
      expect(store.getConversation("self").pending[0]).toMatchObject({ status: "failed", retryable: true });
      expect(outbox(), "still kept for later").toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("drops a text that was deleted on the other side after an earlier attempt went through", async () => {
    h.events.state = "open";
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    h.api.post.mockRejectedValueOnce(new h.ApiError(410, "这条消息已在另一端删除"));
    store.sendText("self", "a password");
    await settle();
    expect(store.getConversation("self").pending).toEqual([]);
    expect(outbox()).toEqual([]);
  });

  it("sends nothing still queued once this page's pairing has ended", async () => {
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    store.sendText("self", "typed under the old pairing");
    store.sendFiles("self", [new File(["x"], "a.jpg")]);
    const { clearConversationData } = await import("../lib/storage");
    clearConversationData();
    h.setState("open");
    await settle();
    expect(h.api.post).not.toHaveBeenCalled();
    expect(h.api.get, "only the first page load, no upload check").toHaveBeenCalledTimes(1);
    expect(h.uploadFile).not.toHaveBeenCalled();
    expect(outbox()).toEqual([]);
  });

  it("sends texts left by an earlier visit without the chat being opened", async () => {
    queue("c-7", "queued");
    h.events.state = "open";
    h.api.post.mockResolvedValue(msg(3, { client_id: "c-7" }));
    const store = await freshStore();
    store.startOutbox();
    await settle();
    expect(h.api.post.mock.calls.map((c) => c[1].client_id)).toEqual(["c-7"]);
    expect(outbox()).toEqual([]);
    // Opening the chat later does not send it again.
    await store.ensureLoaded("self", null);
    await settle();
    expect(h.api.post).toHaveBeenCalledTimes(1);
  });
});

describe("attachments", () => {
  it("asks first, and a file the computer has no room for is never sent", async () => {
    h.events.state = "open";
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    h.api.get.mockRejectedValueOnce(new h.ApiError(507, "电脑磁盘空间不足"));
    store.sendFiles("self", [new File(["x".repeat(10)], "big.mov")]);
    await settle();
    expect(h.api.get).toHaveBeenLastCalledWith("/api/upload-check?size=10", expect.any(Number));
    expect(h.uploadFile).not.toHaveBeenCalled();
    expect(store.getConversation("self").pending[0]).toMatchObject({ status: "failed", error: "电脑磁盘空间不足" });
  });

  it("says when the computer's shared folder is unavailable, and 重试 sends the file once it is back", async () => {
    h.events.state = "open";
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    const why = "电脑上的共享文件夹现在不可用，所在的磁盘可能没有连接";
    h.api.get.mockRejectedValueOnce(new h.ApiError(503, why));
    store.sendFiles("self", [new File(["12345"], "v.mov")]);
    await settle();
    const [p] = store.getConversation("self").pending;
    expect(p).toMatchObject({ status: "failed", error: why, retryable: true });
    expect(h.uploadFile).not.toHaveBeenCalled();
    h.uploadFile.mockReturnValue({ promise: new Promise(() => undefined), abort: () => undefined });
    store.retry("self", p.client_id);
    await settle();
    expect(h.uploadFile).toHaveBeenCalledTimes(1);
  });

  it("tells the computer the file's size, so a body that arrives empty is caught", async () => {
    h.events.state = "open";
    h.uploadFile.mockReturnValue({ promise: new Promise(() => undefined), abort: () => undefined });
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    store.sendFiles("self", [new File(["12345"], "v.mov")]);
    await settle();
    expect(h.uploadFile.mock.calls[0][0]).toBe("/api/messages/upload?client_id=" + store.getConversation("self").pending[0].client_id + "&name=v.mov&size=5");
  });
});

describe("changes made elsewhere", () => {
  it("merges each message once and clears the pending copy", async () => {
    h.events.state = "open";
    h.api.post.mockReturnValue(new Promise(() => undefined));
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    store.sendText("self", "hello");
    const cid = store.getConversation("self").pending[0].client_id;
    h.emit("msg", msg(7, { client_id: cid }));
    h.emit("msg", msg(7, { client_id: cid }));
    expect(store.getConversation("self").messages.map((m) => m.id)).toEqual([7]);
    expect(store.getConversation("self").pending).toEqual([]);
  });

  it("drops deleted messages and everything older than a cleanup", async () => {
    h.api.get.mockResolvedValue({ messages: [msg(1), msg(2), msg(3)], has_more: true });
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    h.emit("msg_deleted", { ids: [2] });
    expect(store.getConversation("self").messages.map((m) => m.id)).toEqual([1, 3]);
    h.emit("pruned", { before: 25 });
    expect(store.getConversation("self").messages.map((m) => m.id)).toEqual([3]);
    expect(store.getConversation("self").hasMore).toBe(false);
  });

  it("drops what a clear removed by id, whatever the clocks said", async () => {
    // Message 5 arrived after the clear, with an earlier time (the computer's clock had been fast).
    h.api.get.mockResolvedValue({ messages: [msg(3, { created_at: 900 }), msg(4, { created_at: 950 }), msg(5, { created_at: 100 })], has_more: false });
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    h.emit("pruned", { before: 0, through: 4 });
    expect(store.getConversation("self").messages.map((m) => m.id)).toEqual([5]);
    h.emit("msg", msg(6));
    h.emit("cleared", { through: 5 });
    expect(store.getConversation("self").messages.map((m) => m.id)).toEqual([6]);
  });

  it("deletes on the computer and removes the message here", async () => {
    h.api.get.mockResolvedValue({ messages: [msg(1), msg(2)], has_more: false });
    const store = await freshStore();
    await store.ensureLoaded("self", null);
    await store.deleteMessage(1, true);
    expect(h.api.del).toHaveBeenCalledWith("/api/messages/1?trash_file=1");
    expect(store.getConversation("self").messages.map((m) => m.id)).toEqual([2]);
  });
});
