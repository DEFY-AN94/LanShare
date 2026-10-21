// The event stream's reconnect rules, with EventSource, fetch and the page
// replaced by stand-ins.

import { afterEach, beforeEach, expect, it, vi } from "vitest";

class FakeEventSource {
  static OPEN = 1;
  static all: FakeEventSource[] = [];
  readyState = 0;
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor() {
    FakeEventSource.all.push(this);
  }
  addEventListener() {}
  close() {}
  open() {
    this.readyState = FakeEventSource.OPEN;
    this.onopen?.();
  }
  fail() {
    this.onerror?.();
  }
}

const windowListeners = new Map<string, () => void>();
let probe: { resolve: (r: { status: number }) => void; reject: (e: unknown) => void };

beforeEach(() => {
  vi.useFakeTimers();
  vi.resetModules();
  FakeEventSource.all = [];
  windowListeners.clear();
  Object.assign(globalThis, {
    window: {
      setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms),
      clearTimeout: (t: number) => clearTimeout(t),
      setInterval: (fn: () => void, ms: number) => setInterval(fn, ms),
      addEventListener: (type: string, fn: () => void) => windowListeners.set(type, fn),
    },
    document: { visibilityState: "visible", addEventListener: () => undefined },
    EventSource: FakeEventSource,
    fetch: vi.fn(() => new Promise((resolve, reject) => (probe = { resolve, reject }))),
  });
});

afterEach(() => vi.useRealTimers());

it("a late probe of a dropped stream does not mark a working new stream offline", async () => {
  const { events } = await import("./events");
  events.start();
  FakeEventSource.all[0].open();
  // Wi-Fi blips: the stream drops and a check on the computer starts, then hangs.
  FakeEventSource.all[0].fail();
  expect(fetch).toHaveBeenCalledTimes(1);
  // Wi-Fi is back: a new stream opens at once.
  windowListeners.get("online")!();
  FakeEventSource.all[1].open();
  expect(events.state).toBe("open");
  // The old check gives up afterwards.
  probe.reject(new TypeError("aborted"));
  await vi.advanceTimersByTimeAsync(0);
  expect(events.state).toBe("open");
});

it("a stream that drops while the computer is unreachable shows offline and tries again", async () => {
  const { events } = await import("./events");
  events.start();
  FakeEventSource.all[0].open();
  FakeEventSource.all[0].fail();
  probe.reject(new TypeError("Load failed"));
  await vi.advanceTimersByTimeAsync(0);
  expect(events.state).toBe("offline");
  await vi.advanceTimersByTimeAsync(2_000);
  expect(FakeEventSource.all).toHaveLength(2);
});
