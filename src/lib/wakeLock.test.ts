// Keeping the screen on during uploads. WebKit grants a wake lock only soon
// after a tap on the page; from the photo library the upload starts a minute
// or more after the tap that opened the picker (seen on a real iPhone).

import { afterEach, beforeEach, expect, it, vi } from "vitest";

type Listener = (e?: unknown) => void;
const docListeners = new Map<string, Set<Listener>>();
let tapping = false;
const released = vi.fn();
const request = vi.fn(async () => {
  if (!tapping) throw new Error("NotAllowedError");
  return { release: async () => released(), addEventListener: () => undefined };
});

vi.stubGlobal("navigator", { wakeLock: { request } });
vi.stubGlobal("document", {
  visibilityState: "visible",
  addEventListener: (type: string, fn: Listener) => {
    if (!docListeners.has(type)) docListeners.set(type, new Set());
    docListeners.get(type)!.add(fn);
  },
  removeEventListener: (type: string, fn: Listener) => docListeners.get(type)?.delete(fn),
});

/** A tap on the page: the document's listeners run, then `then` (the button's own handler). */
function tap(then?: () => void) {
  tapping = true;
  for (const type of ["pointerdown", "touchend", "click"]) docListeners.get(type)?.forEach((fn) => fn());
  then?.();
  tapping = false;
}

function fileInput() {
  const listeners = new Map<string, Listener>();
  return {
    el: {
      addEventListener: (type: string, fn: Listener) => void listeners.set(type, fn),
      removeEventListener: (type: string) => void listeners.delete(type),
      click: vi.fn(),
    } as unknown as HTMLInputElement,
    fire: (type: "change" | "cancel") => listeners.get(type)?.(),
  };
}

const settle = () => vi.advanceTimersByTimeAsync(0);

/** The first tap on each page load asks once and lets go at once; start counting after it. */
async function afterFirstTap() {
  tap();
  await settle();
  request.mockClear();
  released.mockClear();
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.resetModules();
  docListeners.clear();
  request.mockClear();
  released.mockClear();
});

afterEach(() => vi.useRealTimers());

it("the tap that opens the picker keeps the screen on, and the upload takes over without asking again", async () => {
  const { holdScreen, openPicker } = await import("./wakeLock");
  await afterFirstTap();
  const input = fileInput();
  tap(() => openPicker(input.el));
  await settle();
  expect(input.el.click).toHaveBeenCalled();
  expect(request).toHaveBeenCalledTimes(1);

  // iOS converts the video for a minute, then hands it over: no tap now.
  await vi.advanceTimersByTimeAsync(60_000);
  input.fire("change");
  const releaseUpload = holdScreen();
  await settle();
  await vi.advanceTimersByTimeAsync(10_000);
  expect(request).toHaveBeenCalledTimes(1);
  expect(released).not.toHaveBeenCalled(); // still held, by the upload

  releaseUpload();
  await settle();
  expect(released).toHaveBeenCalledTimes(1);
});

it("a lock that was refused is asked for again on the next tap", async () => {
  const { holdScreen } = await import("./wakeLock");
  await afterFirstTap();
  const release = holdScreen(); // an upload starting with no recent tap
  await settle();
  expect(request).toHaveBeenCalledTimes(1); // refused

  tap();
  await settle();
  expect(request).toHaveBeenCalledTimes(2);
  release();
  await settle();
  expect(released).toHaveBeenCalledTimes(1); // the lock from the tap, now let go
});

it("a cancelled picker lets go of the screen", async () => {
  const { openPicker } = await import("./wakeLock");
  await afterFirstTap();
  const input = fileInput();
  tap(() => openPicker(input.el));
  await settle();
  expect(released).not.toHaveBeenCalled();
  input.fire("cancel");
  await settle();
  expect(released).toHaveBeenCalledTimes(1);
});

it("a picker that never answers lets go on the next tap, or after ten minutes", async () => {
  const { openPicker } = await import("./wakeLock");
  await afterFirstTap();
  const first = fileInput();
  tap(() => openPicker(first.el));
  await settle();
  tap(); // back on the page: the picker is gone
  await settle();
  expect(released).toHaveBeenCalledTimes(1);

  const second = fileInput();
  tap(() => openPicker(second.el));
  await settle();
  await vi.advanceTimersByTimeAsync(10 * 60_000);
  expect(released).toHaveBeenCalledTimes(2);
});
