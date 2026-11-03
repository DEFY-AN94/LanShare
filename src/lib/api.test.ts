// What a raw upload puts on the wire, and how it ends when it stalls or is
// cancelled; how a request ends when its reply stops arriving or is not JSON.

import { afterEach, expect, it, vi } from "vitest";

it("sends a copy of the file's bytes, not the picked File itself", async () => {
  const sent: unknown[] = [];
  class FakeXHR {
    upload = {};
    open() {}
    setRequestHeader() {}
    send(body: unknown) {
      sent.push(body);
    }
    abort() {}
  }
  Object.assign(globalThis, {
    XMLHttpRequest: FakeXHR,
    window: { setInterval: () => 0, clearInterval: () => undefined },
    document: { visibilityState: "visible", addEventListener: () => undefined, removeEventListener: () => undefined },
  });
  const { uploadFile } = await import("./api");
  const file = new File(["hello"], "a.txt", { type: "text/plain" });
  uploadFile("/api/files/upload?size=5", file, () => undefined);

  const body = sent[0] as Blob;
  expect(body).not.toBe(file);
  expect(body).toBeInstanceOf(Blob);
  expect(body.type).toBe("text/plain");
  expect(await body.text()).toBe("hello");
});

/** An XHR that sends nothing until the test says so; abort() ends it the way a browser does. */
class StallingXHR {
  static last: StallingXHR;
  upload: { onprogress?: (e: { loaded: number; total: number; lengthComputable: boolean }) => void } = {};
  onabort?: () => void;
  onloadend?: () => void;
  constructor() {
    StallingXHR.last = this;
  }
  open() {}
  setRequestHeader() {}
  send() {}
  abort() {
    this.onabort?.();
    this.onloadend?.();
  }
  progress(loaded: number) {
    this.upload.onprogress?.({ loaded, total: 100, lengthComputable: true });
  }
}

const page = { visibilityState: "visible", onVisible: undefined as undefined | (() => void) };

function stallingUpload() {
  vi.useFakeTimers();
  page.visibilityState = "visible";
  Object.assign(globalThis, {
    XMLHttpRequest: StallingXHR,
    window: { setInterval: (fn: () => void, ms: number) => setInterval(fn, ms), clearInterval: (t: number) => clearInterval(t) },
    document: {
      get visibilityState() {
        return page.visibilityState;
      },
      addEventListener: (_type: string, fn: () => void) => (page.onVisible = fn),
      removeEventListener: () => undefined,
    },
  });
}

afterEach(() => vi.useRealTimers());

it("stops an upload that moved nothing for a minute, as a lost connection the caller tries again", async () => {
  stallingUpload();
  const { uploadFile } = await import("./api");
  let error: unknown;
  void uploadFile("/api/files/upload?size=100", new Blob(["x"]), () => undefined).promise.catch((e) => (error = e));
  await vi.advanceTimersByTimeAsync(50_000);
  StallingXHR.last.progress(40); // still moving
  await vi.advanceTimersByTimeAsync(58_000);
  expect(error).toBeUndefined();
  await vi.advanceTimersByTimeAsync(4_000);
  expect(error).toMatchObject({ status: 0, message: "上传停住了，连上后会自动重试" });
});

it("a cancelled upload ends with -1, so it is removed and not tried again", async () => {
  stallingUpload();
  const { uploadFile } = await import("./api");
  const handle = uploadFile("/api/files/upload?size=100", new Blob(["x"]), () => undefined);
  handle.abort();
  await expect(handle.promise).rejects.toMatchObject({ status: -1, message: "已取消" });
});

it("does not count time with the page hidden, and gives a resumed upload 10 s to move", async () => {
  stallingUpload();
  const { uploadFile } = await import("./api");
  let error: unknown;
  void uploadFile("/api/files/upload?size=100", new Blob(["x"]), () => undefined).promise.catch((e) => (error = e));
  page.visibilityState = "hidden";
  await vi.advanceTimersByTimeAsync(5 * 60_000);
  expect(error).toBeUndefined();
  page.visibilityState = "visible";
  page.onVisible?.();
  await vi.advanceTimersByTimeAsync(8_000);
  expect(error).toBeUndefined();
  await vi.advanceTimersByTimeAsync(4_000);
  expect(error).toMatchObject({ status: 0 });
});

/** A reply with this status whose headers arrive and whose body never does; an abort ends the body read, as in a browser. */
function stallingBody(status: number) {
  vi.useFakeTimers();
  Object.assign(globalThis, {
    window: { setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms), clearTimeout: (t: number) => clearTimeout(t) },
    fetch: vi.fn(async (_url: string, init: RequestInit) => ({
      ok: status >= 200 && status < 300,
      status,
      headers: new Headers({ "content-type": "application/json" }),
      json: () => new Promise((_resolve, reject) => init.signal!.addEventListener("abort", () => reject(new DOMException("The operation was aborted.", "AbortError")))),
    })),
  });
}

it.each([
  ["a page", 200, { status: 0, message: "连接不上电脑，请检查是否在同一个 Wi-Fi" }],
  ["an error", 500, { status: 500, message: "请求失败（500）" }],
])("gives up on %s whose body stops arriving after the headers, as on a lost connection", async (_what, status, expected) => {
  stallingBody(status);
  const { api, ApiError } = await import("./api");
  let error: unknown;
  void api.get("/api/messages?limit=50", 1_000).catch((e) => (error = e));
  await vi.advanceTimersByTimeAsync(999);
  expect(error).toBeUndefined();
  await vi.advanceTimersByTimeAsync(1);
  expect(error).toBeInstanceOf(ApiError);
  expect(error).toMatchObject(expected);
});

it("says so when a reply is not the JSON it claims to be", async () => {
  Object.assign(globalThis, {
    window: { setTimeout: () => 0, clearTimeout: () => undefined },
    fetch: vi.fn(async () => new Response("{not json", { headers: { "content-type": "application/json" } })),
  });
  const { api, ApiError } = await import("./api");
  const reply = api.get("/api/messages?limit=50");
  await expect(reply).rejects.toBeInstanceOf(ApiError);
  await expect(reply).rejects.toMatchObject({ status: 200, message: "服务器返回了无法识别的内容" });
});

it("says so too in Safari, which reports such a reply as a DOMException", async () => {
  Object.assign(globalThis, {
    window: { setTimeout: () => 0, clearTimeout: () => undefined },
    fetch: vi.fn(async () => ({
      ok: true,
      status: 200,
      headers: new Headers({ "content-type": "application/json" }),
      json: () => Promise.reject(new DOMException("The string did not match the expected pattern.", "SyntaxError")),
    })),
  });
  const { api, ApiError } = await import("./api");
  const reply = api.get("/api/messages?limit=50");
  await expect(reply).rejects.toBeInstanceOf(ApiError);
  await expect(reply).rejects.toMatchObject({ status: 200, message: "服务器返回了无法识别的内容" });
});
