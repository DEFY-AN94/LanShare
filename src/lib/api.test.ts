// What a raw upload puts on the wire.

import { expect, it } from "vitest";

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
