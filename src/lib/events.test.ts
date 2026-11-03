// The event stream's reconnect rules, with EventSource, fetch and the page
// replaced by stand-ins.

import { afterEach, beforeEach, expect, it, vi } from "vitest";

class FakeEventSource {
  static OPEN = 1;
  static all: FakeEventSource[] = [];
  readyState = 0;
  closed = false;
  listeners = new Map<string, (e: { data: string }) => void>();
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor() {
    FakeEventSource.all.push(this);
  }
  addEventListener(name: string, fn: (e: { data: string }) => void) {
    this.listeners.set(name, fn);
  }
  close() {
    this.closed = true;
  }
  /** The computer sends an event on this stream. */
  send(name: string, data: unknown = null) {
    this.listeners.get(name)?.({ data: JSON.stringify(data) });
  }
  open() {
    this.readyState = FakeEventSource.OPEN;
    this.onopen?.();
  }
  fail() {
    this.onerror?.();
  }
}

const windowListeners = new Map<string, () => void>();
const page = { visibilityState: "visible", listeners: new Map<string, () => void>() };
/** The user leaves the page or comes back to it. */
function show(visible: boolean) {
  page.visibilityState = visible ? "visible" : "hidden";
  page.listeners.get("visibilitychange")?.();
}
let probe: { resolve: (r: { status: number }) => void; reject: (e: unknown) => void };

beforeEach(() => {
  vi.useFakeTimers();
  vi.resetModules();
  FakeEventSource.all = [];
  windowListeners.clear();
  page.visibilityState = "visible";
  page.listeners.clear();
  Object.assign(globalThis, {
    window: {
      setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms),
      clearTimeout: (t: number) => clearTimeout(t),
      setInterval: (fn: () => void, ms: number) => setInterval(fn, ms),
      addEventListener: (type: string, fn: () => void) => windowListeners.set(type, fn),
    },
    document: {
      get visibilityState() {
        return page.visibilityState;
      },
      addEventListener: (type: string, fn: () => void) => page.listeners.set(type, fn),
    },
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

it("replaces a stream that has gone quiet for 40 s, and fetches what was missed", async () => {
  const { events } = await import("./events");
  const resync = vi.fn();
  events.on("resync", resync);
  events.start();
  FakeEventSource.all[0].open();
  // Pings every 15 s keep it.
  for (let i = 0; i < 3; i++) {
    await vi.advanceTimersByTimeAsync(15_000);
    FakeEventSource.all[0].send("ping");
  }
  expect(FakeEventSource.all).toHaveLength(1);
  // Then nothing arrives, though it still looks open.
  await vi.advanceTimersByTimeAsync(45_000);
  expect(FakeEventSource.all).toHaveLength(2);
  expect(FakeEventSource.all[0].closed).toBe(true);
  expect(events.state).toBe("connecting");
  FakeEventSource.all[1].open();
  expect(resync).toHaveBeenCalledTimes(1);
});

it("reconnects at once when the page comes back after more than 10 s, and keeps the stream after a short look away", async () => {
  const { events } = await import("./events");
  events.start();
  FakeEventSource.all[0].open();
  show(false);
  await vi.advanceTimersByTimeAsync(5_000);
  FakeEventSource.all[0].send("ping");
  show(true);
  expect(FakeEventSource.all).toHaveLength(1);

  show(false);
  await vi.advanceTimersByTimeAsync(11_000);
  show(true);
  expect(FakeEventSource.all).toHaveLength(2);
  expect(FakeEventSource.all[0].closed).toBe(true);
});

it("a phone page closes its stream while sharing is paused and checks again after 4 s; the computer's window keeps it", async () => {
  const { events } = await import("./events");
  events.start();
  FakeEventSource.all[0].open();
  FakeEventSource.all[0].send("paused", { paused: true });
  expect(events.state).toBe("paused");
  expect(FakeEventSource.all[0].closed).toBe(true);
  await vi.advanceTimersByTimeAsync(4_000);
  expect(FakeEventSource.all).toHaveLength(2);

  vi.resetModules();
  FakeEventSource.all = [];
  const host = (await import("./events")).events;
  host.isDeviceStream = false;
  host.start();
  FakeEventSource.all[0].open();
  FakeEventSource.all[0].send("paused", { paused: true });
  expect(host.state).toBe("open");
  expect(FakeEventSource.all[0].closed).toBe(false);
});

it("gives up on a stream still connecting after 6 s, checks the computer and tries again", async () => {
  const { events } = await import("./events");
  events.start();
  await vi.advanceTimersByTimeAsync(5_999);
  expect(fetch).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  expect(FakeEventSource.all[0].closed).toBe(true);
  expect(fetch).toHaveBeenCalledTimes(1);
  probe.resolve({ status: 200 });
  await vi.advanceTimersByTimeAsync(1_000);
  expect(FakeEventSource.all).toHaveLength(2);
  expect(events.state).toBe("connecting");
});
