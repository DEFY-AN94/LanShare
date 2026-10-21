// Upload queue for the file browser. Lives at module level so uploads keep
// going when the user switches tabs or folders.

import { useSyncExternalStore } from "react";
import { type ApiError, api, qs, uploadFile, type UploadHandle } from "../lib/api";
import { events } from "../lib/events";
import { newClientId } from "../lib/format";
import type { FileEntry } from "../lib/types";
import { holdScreen } from "../lib/wakeLock";

export interface UploadItem {
  /** Also the upload ID: a retry sends the same one, so the computer keeps one copy. */
  id: string;
  name: string;
  size: number;
  loaded: number;
  dir: string;
  status: "queued" | "uploading" | "done" | "failed";
  error?: string;
  /** False when trying again cannot help (no permission, bad name, too big for the disk format). */
  retryable?: boolean;
  /** Automatic retries used after the connection came back. */
  autoRetries: number;
  startedAt?: number;
  speed: number;
  file: File;
}

const MAX_PARALLEL = 2;
const MAX_AUTO_RETRIES = 3;
/** Waits before each automatic retry while the connection is up. */
const RETRY_DELAYS_MS = [2_000, 5_000, 10_000];
/** "Would it fit?" asked before a file is sent. */
const CHECK_TIMEOUT_MS = 8_000;
let items: UploadItem[] = [];
const handles = new Map<string, UploadHandle<FileEntry>>();
const listeners = new Set<() => void>();
const doneListeners = new Set<(dir: string) => void>();
let releaseScreen: (() => void) | null = null;
let subscribed = false;

function emit() {
  items = [...items];
  listeners.forEach((fn) => fn());
}

function patch(id: string, p: Partial<UploadItem>) {
  items = items.map((it) => (it.id === id ? { ...it, ...p } : it));
  listeners.forEach((fn) => fn());
}

/** Network errors, timeouts and server trouble are worth another try; 4xx refusals are not. */
function retryableStatus(status: number) {
  return status <= 0 || status === 408 || status === 429 || status === 507 || status >= 500;
}

function updateScreenHold() {
  const active = handles.size > 0;
  if (active && !releaseScreen) releaseScreen = holdScreen();
  else if (!active && releaseScreen) {
    releaseScreen();
    releaseScreen = null;
  }
}

function pump() {
  const running = items.filter((i) => i.status === "uploading").length;
  let slots = MAX_PARALLEL - running;
  for (const it of items) {
    if (slots <= 0) break;
    if (it.status !== "queued") continue;
    slots--;
    void start(it);
  }
}

function failed(it: UploadItem, e: ApiError) {
  patch(it.id, { status: "failed", error: e.message, retryable: retryableStatus(e.status) });
  // Still connected (a stalled upload, a busy computer): try again shortly.
  // Otherwise the retry waits for the connection to come back.
  const current = items.find((x) => x.id === it.id);
  if (current && retryableStatus(e.status) && events.state === "open" && current.autoRetries < MAX_AUTO_RETRIES) {
    window.setTimeout(retryAfterReconnect, RETRY_DELAYS_MS[current.autoRetries] ?? 10_000);
  }
  emit();
}

async function start(it: UploadItem) {
  const startedAt = performance.now();
  patch(it.id, { status: "uploading", startedAt, loaded: 0, error: undefined });
  try {
    // Ask first: a refusal (disk space, a 4 GB limit) then arrives as a clear
    // message instead of a connection the computer had to cut.
    await api.get(`/api/upload-check${qs({ dir: it.dir, size: it.size })}`, CHECK_TIMEOUT_MS);
  } catch (e) {
    failed(it, e as ApiError);
    pump();
    return;
  }
  if (!items.some((x) => x.id === it.id && x.status === "uploading")) {
    pump(); // cancelled while asking
    return;
  }
  const handle = uploadFile<FileEntry>(`/api/files/upload${qs({ dir: it.dir, name: it.name, upload_id: it.id, size: it.size })}`, it.file, (loaded) => {
    const secs = (performance.now() - startedAt) / 1000;
    patch(it.id, { loaded, speed: secs > 0.3 ? loaded / secs : 0 });
  });
  handles.set(it.id, handle);
  updateScreenHold();
  handle.promise
    .then(() => {
      patch(it.id, { status: "done", loaded: it.size });
      doneListeners.forEach((fn) => fn(it.dir));
    })
    .catch((e: ApiError) => {
      if (e.status === -1) {
        items = items.filter((x) => x.id !== it.id);
        emit();
      } else failed(it, e);
    })
    .finally(() => {
      handles.delete(it.id);
      updateScreenHold();
      pump();
    });
}

/** When the connection comes back, failed uploads try again by themselves, a few times at most. */
function retryAfterReconnect() {
  let changed = false;
  items = items.map((it) => {
    if (it.status !== "failed" || it.retryable === false || it.autoRetries >= MAX_AUTO_RETRIES) return it;
    changed = true;
    return { ...it, status: "queued", error: undefined, loaded: 0, autoRetries: it.autoRetries + 1 };
  });
  if (changed) {
    emit();
    pump();
  }
}

function ensureSubscribed() {
  if (subscribed) return;
  subscribed = true;
  events.on("resync", retryAfterReconnect);
  window.addEventListener("online", retryAfterReconnect);
}

export const uploads = {
  add(files: File[], dir: string) {
    ensureSubscribed();
    for (const file of files) {
      items.push({ id: newClientId(), name: file.name, size: file.size, loaded: 0, dir, status: "queued", speed: 0, autoRetries: 0, file });
    }
    emit();
    pump();
  },
  cancel(id: string) {
    const h = handles.get(id);
    if (h) h.abort();
    else {
      // Queued, or still asking whether it fits.
      items = items.filter((x) => x.id !== id);
      emit();
      pump();
    }
  },
  retry(id: string) {
    patch(id, { status: "queued", error: undefined, loaded: 0 });
    pump();
  },
  clearFinished() {
    items = items.filter((x) => x.status === "queued" || x.status === "uploading");
    emit();
  },
  onDone(fn: (dir: string) => void) {
    doneListeners.add(fn);
    return () => doneListeners.delete(fn);
  },
};

export function useUploads(): UploadItem[] {
  return useSyncExternalStore(
    (fn) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    () => items,
  );
}
