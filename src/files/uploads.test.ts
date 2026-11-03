// The Files upload queue: automatic retries, refusals, cancelling and keeping
// the screen on, with the network, the event stream and the wake lock
// replaced by stand-ins.

import { afterEach, beforeEach, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
  class ApiError extends Error {
    status: number;
    constructor(status: number, message: string) {
      super(message);
      this.status = status;
    }
  }
  const handlers = new Map<string, Set<(data: unknown) => void>>();
  const release = vi.fn();
  return {
    ApiError,
    handlers,
    events: {
      state: "open",
      on(name: string, fn: (data: unknown) => void) {
        if (!handlers.has(name)) handlers.set(name, new Set());
        handlers.get(name)!.add(fn);
        return () => handlers.get(name)!.delete(fn);
      },
    },
    emit: (name: string) => handlers.get(name)?.forEach((fn) => fn(null)),
    api: { get: vi.fn() },
    uploadFile: vi.fn(),
    release,
    holdScreen: vi.fn(() => release),
  };
});

vi.mock("../lib/events", () => ({ events: h.events }));
vi.mock("../lib/wakeLock", () => ({ holdScreen: h.holdScreen }));
vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return { api: h.api, qs: actual.qs, uploadFile: h.uploadFile, ApiError: h.ApiError };
});
// useUploads outside a component: read the store directly.
vi.mock("react", () => ({ useSyncExternalStore: (_subscribe: unknown, get: () => unknown) => get() }));

const windowListeners = new Map<string, () => void>();
Object.assign(globalThis, {
  window: {
    setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms),
    addEventListener: (type: string, fn: () => void) => windowListeners.set(type, fn),
  },
});

/** One upload request: settle it by hand, or let `abort` cancel it the way the XHR does. */
function request() {
  let resolve!: (v: unknown) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise((res, rej) => ((resolve = res), (reject = rej)));
  return { promise, abort: () => reject(new h.ApiError(-1, "已取消")), resolve, reject };
}
let sent: ReturnType<typeof request>[] = [];

const file = (name: string) => new File(["0123456789"], name);
const uploadIds = () => h.uploadFile.mock.calls.map((c) => new URL(c[0] as string, "https://example.com").searchParams.get("upload_id"));
const settle = () => vi.advanceTimersByTimeAsync(0);

async function freshQueue() {
  vi.resetModules();
  return import("./uploads");
}

beforeEach(() => {
  vi.useFakeTimers();
  h.handlers.clear();
  windowListeners.clear();
  h.events.state = "open";
  h.api.get.mockReset().mockResolvedValue(undefined);
  sent = [];
  h.uploadFile.mockReset().mockImplementation(() => {
    const r = request();
    sent.push(r);
    return r;
  });
  h.holdScreen.mockClear();
  h.release.mockClear();
});

afterEach(() => vi.useRealTimers());

it("tries a stalled upload again by itself with the same upload ID, so the computer keeps one copy", async () => {
  const { uploads, useUploads } = await freshQueue();
  uploads.add([file("a.mov")], "Videos");
  await settle();
  sent[0].reject(new h.ApiError(0, "上传停住了，连上后会自动重试"));
  await settle();
  expect(useUploads()[0]).toMatchObject({ status: "failed", retryable: true });
  await vi.advanceTimersByTimeAsync(2_000);
  expect(h.uploadFile).toHaveBeenCalledTimes(2);
  // 重试 after a busy computer: the same ID again.
  sent[1].reject(new h.ApiError(503, "电脑端已暂停共享"));
  await settle();
  uploads.retry(useUploads()[0].id);
  await settle();
  expect(uploadIds()).toEqual([useUploads()[0].id, useUploads()[0].id, useUploads()[0].id]);
});

it("tries again by itself at most 3 times, also after a reconnect, then offers 重试 and lets the screen go", async () => {
  const { uploads, useUploads } = await freshQueue();
  uploads.add([file("a.mov")], "");
  for (const wait of [2_000, 5_000, 10_000]) {
    await settle();
    sent[sent.length - 1].reject(new h.ApiError(0, "上传中断，请检查网络"));
    await vi.advanceTimersByTimeAsync(wait);
  }
  await settle();
  sent[3].reject(new h.ApiError(0, "上传中断，请检查网络"));
  await settle();
  expect(useUploads()[0]).toMatchObject({ status: "failed", retryable: true, autoRetries: 3 });
  expect(h.holdScreen).toHaveBeenCalledTimes(1);
  expect(h.release).toHaveBeenCalledTimes(1);
  h.emit("resync");
  windowListeners.get("online")!();
  await vi.advanceTimersByTimeAsync(60_000);
  expect(h.uploadFile).toHaveBeenCalledTimes(4);
});

it("lets the screen go when a failed upload is cleared before its retry was due", async () => {
  const { uploads } = await freshQueue();
  uploads.add([file("a.mov")], "");
  await settle();
  sent[0].reject(new h.ApiError(0, "上传停住了，连上后会自动重试"));
  await settle();
  uploads.clearFinished();
  expect(h.release).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(2_000);
  expect(h.uploadFile).toHaveBeenCalledTimes(1);
  expect(h.release).toHaveBeenCalledTimes(1);
});

it("keeps the screen on across a short drop, so the retry after reconnecting still has it", async () => {
  const { uploads, useUploads } = await freshQueue();
  uploads.add([file("a.mov")], "");
  await settle();
  h.events.state = "connecting";
  sent[0].reject(new h.ApiError(0, "上传中断，请检查网络"));
  await vi.advanceTimersByTimeAsync(3_000);
  expect(h.uploadFile).toHaveBeenCalledTimes(1);
  h.events.state = "open";
  h.emit("resync");
  await settle();
  expect(uploadIds()).toEqual([useUploads()[0].id, useUploads()[0].id]);
  sent[1].resolve({});
  await vi.advanceTimersByTimeAsync(30_000);
  expect(useUploads()[0].status).toBe("done");
  expect(h.holdScreen).toHaveBeenCalledTimes(1);
  expect(h.release).toHaveBeenCalledTimes(1);
});

it("waits for the connection before trying again, and lets the screen go after 30 s", async () => {
  const { uploads, useUploads } = await freshQueue();
  uploads.add([file("a.mov")], "");
  await settle();
  h.events.state = "offline";
  sent[0].reject(new h.ApiError(0, "上传中断，请检查网络"));
  await vi.advanceTimersByTimeAsync(29_999);
  expect(h.release).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  expect(h.release).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(60_000);
  expect(h.uploadFile).toHaveBeenCalledTimes(1);

  h.events.state = "open";
  h.emit("resync");
  await settle();
  expect(uploadIds()).toEqual([useUploads()[0].id, useUploads()[0].id]);
});

it("tries a check the computer could not answer again by itself, with the screen kept on", async () => {
  const { uploads, useUploads } = await freshQueue();
  h.api.get.mockRejectedValueOnce(new h.ApiError(503, "电脑上的共享文件夹现在不可用，所在的磁盘可能没有连接"));
  uploads.add([file("a.mov")], "");
  await settle();
  expect(useUploads()[0]).toMatchObject({ status: "failed", retryable: true });
  expect(h.uploadFile).not.toHaveBeenCalled();
  // Taken from the start, and kept while the retry is due.
  expect(h.holdScreen).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(2_000);
  expect(h.api.get).toHaveBeenCalledTimes(2);
  expect(h.release).not.toHaveBeenCalled();
  sent[0].resolve({});
  await settle();
  expect(useUploads()[0].status).toBe("done");
  expect(h.holdScreen).toHaveBeenCalledTimes(1);
  expect(h.release).toHaveBeenCalledTimes(1);
});

it("does not try a refused upload again, by itself or after a reconnect", async () => {
  const { uploads, useUploads } = await freshQueue();
  h.api.get.mockRejectedValueOnce(new h.ApiError(403, "电脑上没有权限访问这个位置"));
  uploads.add([file("a.mov"), file("b.mov")], "");
  await settle();
  sent[0].reject(new h.ApiError(409, "已存在同名的文件或文件夹"));
  await settle();
  expect(useUploads().map((i) => [i.status, i.retryable])).toEqual([
    ["failed", false],
    ["failed", false],
  ]);
  await vi.advanceTimersByTimeAsync(30_000);
  h.emit("resync");
  windowListeners.get("online")!();
  await settle();
  expect(h.api.get).toHaveBeenCalledTimes(2);
  expect(h.uploadFile).toHaveBeenCalledTimes(1);
  expect(h.release).toHaveBeenCalledTimes(1);
});

it.each(["it fits", "the check fails"])("a file cancelled while the computer is asked whether it fits is never sent or tried again (%s)", async (answer) => {
  const { uploads, useUploads } = await freshQueue();
  let fits!: () => void;
  let fails!: (e: unknown) => void;
  h.api.get.mockImplementationOnce(() => new Promise<void>((res, rej) => ((fits = res), (fails = rej))));
  uploads.add([file("a.mov")], "");
  await settle();
  expect(h.holdScreen).toHaveBeenCalledTimes(1);
  uploads.cancel(useUploads()[0].id);
  expect(useUploads()).toEqual([]);
  expect(h.release).toHaveBeenCalledTimes(1);
  if (answer === "it fits") fits();
  else fails(new h.ApiError(0, "连接不上电脑，请检查是否在同一个 Wi-Fi"));
  await vi.advanceTimersByTimeAsync(60_000);
  expect(h.api.get).toHaveBeenCalledTimes(1);
  expect(h.uploadFile).not.toHaveBeenCalled();
  expect(h.holdScreen).toHaveBeenCalledTimes(1);
});

it("keeps the screen on from the first file to the last, across a check and an automatic retry", async () => {
  const { uploads, useUploads } = await freshQueue();
  let fitsC!: () => void;
  h.api.get
    .mockResolvedValueOnce(undefined)
    .mockResolvedValueOnce(undefined)
    .mockImplementationOnce(() => new Promise<void>((r) => (fitsC = r)));
  uploads.add([file("a.mov"), file("b.mov"), file("c.mov")], "");
  await settle();
  sent[1].resolve({});
  await settle(); // c.mov is being checked
  sent[0].resolve({});
  await settle(); // a.mov is done too: nothing is sending right now
  fitsC();
  await settle();
  sent[2].reject(new h.ApiError(0, "上传停住了，连上后会自动重试"));
  await vi.advanceTimersByTimeAsync(2_000);
  expect(h.release).not.toHaveBeenCalled();
  sent[3].resolve({});
  await settle();
  expect(useUploads().map((i) => i.status)).toEqual(["done", "done", "done"]);
  expect(h.holdScreen).toHaveBeenCalledTimes(1);
  expect(h.release).toHaveBeenCalledTimes(1);
});

it("lets the screen go when the last upload is cancelled", async () => {
  const { uploads, useUploads } = await freshQueue();
  uploads.add([file("a.mov")], "");
  await settle();
  uploads.cancel(useUploads()[0].id);
  await settle();
  expect(useUploads()).toEqual([]);
  expect(h.release).toHaveBeenCalledTimes(1);
});
