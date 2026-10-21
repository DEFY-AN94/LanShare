// What stays on the phone after unpairing or pairing again.

import { beforeEach, expect, it, vi } from "vitest";

const storage = new Map<string, string>();
const listeners = new Map<string, (e: { key: string | null }) => void>();
const reload = vi.fn();
Object.assign(globalThis, {
  window: {
    localStorage: {
      getItem: (k: string) => storage.get(k) ?? null,
      setItem: (k: string, v: string) => void storage.set(k, v),
      removeItem: (k: string) => void storage.delete(k),
      key: (i: number) => [...storage.keys()][i] ?? null,
      get length() {
        return storage.size;
      },
    },
    addEventListener: (type: string, fn: (e: { key: string | null }) => void) => void listeners.set(type, fn),
    location: { reload },
  },
});

beforeEach(() => {
  storage.clear();
  listeners.clear();
  reload.mockReset();
  vi.resetModules();
});

it("after unpairing, nothing that pairing typed or queued is written again until the page reloads", async () => {
  const { clearConversationData, saveLocal } = await import("./storage");
  saveLocal("draft.self", "half a sentence");
  saveLocal("outbox", [{ client_id: "c-1", text: "queued" }]);
  saveLocal("files.sort", "size");
  clearConversationData();
  expect([...storage.keys()]).toEqual(["lanshare.files.sort"]);

  // The composer saves its draft once more as the page goes away.
  saveLocal("draft.self", "half a sentence");
  saveLocal("outbox", [{ client_id: "c-1", text: "queued" }]);
  saveLocal("files.sort", "name");
  expect([...storage.keys()]).toEqual(["lanshare.files.sort"]);
  expect(storage.get("lanshare.files.sort")).toBe('"name"');
});

it("pairing again in another tab stops this tab and reloads it", async () => {
  const { conversationCleared, saveLocal, startNewPairing } = await import("./storage");
  saveLocal("draft.self", "from the old pairing");
  listeners.get("storage")!({ key: "lanshare.files.sort" });
  expect(conversationCleared()).toBe(false);

  // What the other tab's startNewPairing() writes reaches this tab as a storage event.
  listeners.get("storage")!({ key: "lanshare.paired" });
  expect(conversationCleared()).toBe(true);
  expect(reload).toHaveBeenCalledTimes(1);
  saveLocal("draft.self", "written back as the page goes away");
  expect(storage.get("lanshare.draft.self")).toBe('"from the old pairing"');

  startNewPairing();
  expect(storage.has("lanshare.draft.self")).toBe(false);
  expect(storage.has("lanshare.paired")).toBe(true);
});
