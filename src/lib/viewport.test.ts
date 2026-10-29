// Putting the page back when iOS leaves it scrolled past its content after the
// phone turns: below it (seen in the iOS Simulator) or above its top (seen on a
// real iPhone whose status bar moves from the top edge to the side).

import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { installViewportFix } from "./viewport";

type Listener = () => void;
let listeners: Map<string, Set<Listener>>;
let vv: { height: number; offsetTop: number; scale: number; addEventListener: (type: string, fn: Listener) => void };
let root: { clientHeight: number; scrollHeight: number; style: { setProperty: ReturnType<typeof vi.fn> }; classList: { toggle: ReturnType<typeof vi.fn> } };
let orientation: { type: string; addEventListener: (type: string, fn: Listener) => void };
let win: { innerWidth: number; scrollY: number; scrollTo: ReturnType<typeof vi.fn> } & Record<string, unknown>;

const on = (target: string) => (type: string, fn: Listener) => {
  const key = `${target}:${type}`;
  if (!listeners.has(key)) listeners.set(key, new Set());
  listeners.get(key)!.add(fn);
};
const fire = (key: string) => listeners.get(key)?.forEach((fn) => fn());

/** The wide orientation as measured on the phone: visible 669 px, layout 587 px, scrolled to -82. */
function stuckAboveTop() {
  win.innerWidth = 867;
  win.scrollY = -82;
  orientation.type = "landscape-secondary";
  root.clientHeight = root.scrollHeight = 587;
  Object.assign(vv, { height: 669, offsetTop: -82, scale: 1 });
}

/** The narrow orientation, where nothing is wrong. */
function narrow() {
  win.innerWidth = 669;
  win.scrollY = 0;
  orientation.type = "portrait-secondary";
  root.clientHeight = root.scrollHeight = 869;
  Object.assign(vv, { height: 869, offsetTop: 0, scale: 1 });
}

beforeEach(() => {
  vi.useFakeTimers();
  listeners = new Map();
  vv = { height: 869, offsetTop: 0, scale: 1, addEventListener: on("vv") };
  root = { clientHeight: 869, scrollHeight: 869, style: { setProperty: vi.fn() }, classList: { toggle: vi.fn() } };
  orientation = { type: "portrait-secondary", addEventListener: on("orientation") };
  win = {
    innerWidth: 669,
    scrollY: 0,
    scrollTo: vi.fn(),
    visualViewport: vv,
    screen: { orientation },
    addEventListener: on("window"),
    setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms),
    clearTimeout: (id: number) => clearTimeout(id),
  };
  vi.stubGlobal("window", win);
  vi.stubGlobal("document", { documentElement: root, addEventListener: on("document") });
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("scrolls back down from above the top only once things settle", () => {
  stuckAboveTop();
  installViewportFix();
  vi.advanceTimersByTime(499);
  expect(win.scrollTo).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1);
  expect(win.scrollTo).toHaveBeenCalledExactlyOnceWith(0, 0);
});

it("waits for the turn to end: every viewport event restarts the wait", () => {
  installViewportFix();
  stuckAboveTop();
  fire("window:resize");
  vi.advanceTimersByTime(400);
  fire("vv:scroll");
  vi.advanceTimersByTime(400);
  expect(win.scrollTo).not.toHaveBeenCalled();
  vi.advanceTimersByTime(100);
  expect(win.scrollTo).toHaveBeenCalledExactlyOnceWith(0, 0);
});

it("tries once while iOS keeps the page above its top, so the page does not jump every half second", () => {
  installViewportFix();
  stuckAboveTop();
  fire("window:resize");
  for (let i = 0; i < 5; i++) {
    vi.advanceTimersByTime(500);
    // iOS puts the position back and says so with a scroll event.
    fire("vv:scroll");
  }
  expect(win.scrollTo).toHaveBeenCalledTimes(1);
  // Turning the phone narrow and back to wide is a new chance.
  narrow();
  fire("window:resize");
  vi.advanceTimersByTime(500);
  stuckAboveTop();
  fire("window:resize");
  vi.advanceTimersByTime(500);
  expect(win.scrollTo).toHaveBeenCalledTimes(2);
});

it("tries again after turning the phone round, which keeps the width", () => {
  installViewportFix();
  stuckAboveTop();
  fire("window:resize");
  vi.advanceTimersByTime(500);
  expect(win.scrollTo).toHaveBeenCalledTimes(1);
  // Still above the top, now upside down: only the orientation says so.
  orientation.type = "landscape-primary";
  fire("orientation:change");
  vi.advanceTimersByTime(500);
  expect(win.scrollTo).toHaveBeenCalledTimes(2);
});

it("fixes it again later at the same width once the first scroll held", () => {
  installViewportFix();
  stuckAboveTop();
  fire("window:resize");
  vi.advanceTimersByTime(500);
  win.scrollY = 0;
  fire("vv:scroll");
  vi.advanceTimersByTime(500);
  win.scrollY = -82;
  fire("vv:resize");
  vi.advanceTimersByTime(500);
  expect(win.scrollTo).toHaveBeenCalledTimes(2);
});

it("leaves the page alone while the keyboard is up or the page is zoomed", () => {
  installViewportFix();
  // Above the top, and below the end of content with some room to scroll.
  for (const scrollY of [-82, 500]) {
    stuckAboveTop();
    root.scrollHeight = 1000;
    win.scrollY = scrollY;
    Object.assign(vv, { height: 300, offsetTop: 0 });
    fire("vv:resize");
    vi.advanceTimersByTime(500);
    Object.assign(vv, { height: 669, scale: 1.5 });
    fire("vv:resize");
    vi.advanceTimersByTime(500);
  }
  expect(win.scrollTo).not.toHaveBeenCalled();
});

it("still scrolls back up from below the end of content at once", () => {
  installViewportFix();
  root.scrollHeight = 1000;
  win.scrollY = 200;
  fire("window:resize");
  expect(win.scrollTo).toHaveBeenCalledExactlyOnceWith(0, 131);
});

it("pins the app to the visual viewport and flags the keyboard", () => {
  installViewportFix();
  Object.assign(vv, { height: 500, offsetTop: 20 });
  fire("vv:resize");
  expect(root.style.setProperty).toHaveBeenLastCalledWith("--app-top", "20px");
  expect(root.style.setProperty).toHaveBeenCalledWith("--app-height", "500px");
  expect(root.classList.toggle).toHaveBeenLastCalledWith("keyboard-open", true);
});
