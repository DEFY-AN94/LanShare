// Keeps the phone's screen on while uploads run: when iOS locks the screen,
// Safari suspends the page and the upload stops. Counted, so the chat and
// the file browser can both hold it. The browser releases the lock whenever
// the page is hidden, so it is taken again when the page comes back.
//
// WebKit grants a wake lock only soon after a tap on the page. An upload
// starts after the file picker closes, and from the photo library that can
// be a minute or more later, since iOS converts videos first: on a real
// iPhone the request then failed and the screen locked mid-upload. So the
// tap that opens the picker takes the hold (openPicker) and the uploads take
// it over. While a hold is wanted but the lock is missing (refused, or not
// granted again after the page was hidden), any tap asks again. The first
// tap on each page load also asks once and lets go at once.

type Sentinel = { release: () => Promise<void>; addEventListener?: (type: "release", fn: () => void) => void };
type WakeLockApi = { request: (type: "screen") => Promise<Sentinel> };

let holders = 0;
let sentinel: Sentinel | null = null;
let requesting = false;
let listening = false;

const api = (): WakeLockApi | undefined => (navigator as Navigator & { wakeLock?: WakeLockApi }).wakeLock;

async function acquire() {
  const wl = api();
  if (!wl || sentinel || requesting || holders === 0 || document.visibilityState !== "visible") return;
  requesting = true;
  try {
    const s = await wl.request("screen");
    if (holders === 0) {
      void s.release();
      return;
    }
    sentinel = s;
    // Dropped by the browser (page hidden, power saving): ask again if still needed.
    s.addEventListener?.("release", () => {
      if (sentinel === s) sentinel = null;
      if (holders > 0 && document.visibilityState === "visible") void acquire();
    });
  } catch {
    // Refused (battery saver, no recent tap): the upload still runs, and the next tap asks again.
  } finally {
    requesting = false;
  }
}

function drop() {
  const s = sentinel;
  sentinel = null;
  void s?.release().catch(() => undefined);
}

let primed = false;

/** Asks with the tap's permission: for a hold that lacks the lock, or once per page load. */
function onTap() {
  const wl = api();
  if (!wl) return;
  // acquire() calls request() before its first await, so it still counts as part of the tap.
  if (holders > 0) return void acquire();
  if (primed) return;
  primed = true;
  wl.request("screen").then(
    (s) => void s.release(),
    () => undefined,
  );
}

if (typeof document !== "undefined") {
  for (const type of ["click", "touchend"] as const) document.addEventListener(type, onTap, { capture: true });
}

/** Keeps the screen on until the returned function is called. */
export function holdScreen(): () => void {
  if (!listening) {
    listening = true;
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") void acquire();
      else sentinel = null; // released by the browser
    });
  }
  holders++;
  void acquire();
  let released = false;
  return () => {
    if (released) return;
    released = true;
    holders = Math.max(0, holders - 1);
    if (holders === 0) drop();
  };
}

/** After files are chosen: time for the uploads to take the hold over (the file browser asks the computer first). */
const HANDOVER_MS = 10_000;
/** A picker that never answers (no `cancel` event) lets go after this. */
const PICKER_MAX_MS = 10 * 60_000;

let endPickerHold: (() => void) | null = null;

/**
 * Opens a file picker; call it from the tap. The screen stays on until the
 * chosen files are handed to the uploads, the picker is cancelled, or the
 * page is tapped again.
 */
export function openPicker(input: HTMLInputElement | null) {
  if (!input) return;
  endPickerHold?.();
  const release = holdScreen();
  const timers: ReturnType<typeof setTimeout>[] = [];
  const end = () => {
    timers.forEach(clearTimeout);
    input.removeEventListener("change", chosen);
    input.removeEventListener("cancel", end);
    document.removeEventListener("pointerdown", end, true);
    if (endPickerHold === end) endPickerHold = null;
    release();
  };
  const chosen = () => timers.push(setTimeout(end, HANDOVER_MS));
  input.addEventListener("change", chosen);
  input.addEventListener("cancel", end);
  // Only a later tap ends it, not the one that opened the picker.
  timers.push(setTimeout(() => document.addEventListener("pointerdown", end, true), 0));
  timers.push(setTimeout(end, PICKER_MAX_MS));
  endPickerHold = end;
  input.click();
}
