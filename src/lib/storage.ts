// Small values kept on this device (drafts, unsent messages, preferences).
// localStorage can be missing or full (private browsing), so every call is
// best effort and never throws.

const PREFIX = "lanshare.";
/** Changes each time this phone pairs again (see startNewPairing). */
const PAIRED_KEY = `${PREFIX}paired`;

export function loadLocal<T>(key: string, fallback: T): T {
  try {
    const raw = window.localStorage.getItem(PREFIX + key);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

/** Set when this pairing's data is cleared; until the page reloads, it is not written again. */
let wiped = false;
const isConversationKey = (key: string) => key.startsWith("draft.") || key.startsWith("outbox");

/** True once this page's pairing ended: what it still holds must not be saved or sent. */
export const conversationCleared = () => wiped;

/** Returns whether the value was stored. */
export function saveLocal(key: string, value: unknown): boolean {
  // A draft or queued text saved as the page goes away after unpairing.
  if (wiped && isConversationKey(key)) return false;
  try {
    window.localStorage.setItem(PREFIX + key, JSON.stringify(value));
    return true;
  } catch {
    // full or unavailable: it simply is not kept
    return false;
  }
}

/** Whether `key` is stored; undefined when storage cannot be read. */
export function hasLocal(key: string): boolean | undefined {
  try {
    return window.localStorage.getItem(PREFIX + key) !== null;
  } catch {
    return undefined;
  }
}

/** Calls `fn` with the rest of the key when another tab removes an entry under `prefix`. */
export function onRemovedElsewhere(prefix: string, fn: (rest: string) => void) {
  if (typeof window === "undefined" || typeof window.addEventListener !== "function") return;
  window.addEventListener("storage", (e) => {
    if (e.key?.startsWith(PREFIX + prefix) && e.newValue === null) fn(e.key.slice(PREFIX.length + prefix.length));
  });
}

/** Every value stored under a key that starts with `prefix`. */
export function loadLocalAll<T>(prefix: string): T[] {
  const out: T[] = [];
  try {
    for (let i = 0; i < window.localStorage.length; i++) {
      const k = window.localStorage.key(i);
      if (!k?.startsWith(PREFIX + prefix)) continue;
      try {
        out.push(JSON.parse(window.localStorage.getItem(k) ?? "null") as T);
      } catch {
        // a broken entry is skipped
      }
    }
  } catch {
    // unavailable
  }
  return out;
}

export function removeLocal(key: string) {
  try {
    window.localStorage.removeItem(PREFIX + key);
  } catch {
    // unavailable
  }
}

/** On unpairing: nothing this pairing typed or queued stays on the phone. */
export function clearConversationData() {
  wiped = true;
  try {
    const keys: string[] = [];
    for (let i = 0; i < window.localStorage.length; i++) {
      const k = window.localStorage.key(i);
      if (k?.startsWith(PREFIX) && isConversationKey(k.slice(PREFIX.length))) keys.push(k);
    }
    keys.forEach((k) => window.localStorage.removeItem(k));
  } catch {
    // unavailable
  }
}

/**
 * After this phone paired again: clears what the earlier pairing left, and
 * tells other open tabs of this site, which still belong to it, to reload.
 */
export function startNewPairing() {
  clearConversationData();
  try {
    window.localStorage.setItem(PAIRED_KEY, String(Date.now()));
  } catch {
    // unavailable: other tabs find out at their next request instead
  }
}

// Another tab paired this phone again. The pairing cookie is shared, so this
// tab would now save its old draft and send its queued texts under the new
// pairing: stop that, and reload into the new one.
if (typeof window !== "undefined" && typeof window.addEventListener === "function") {
  window.addEventListener("storage", (e) => {
    if (e.key !== PAIRED_KEY) return;
    wiped = true;
    window.location.reload();
  });
}
