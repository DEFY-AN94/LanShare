// What a raw upload puts on the wire, and how it ends when it stalls or is cancelled.

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
