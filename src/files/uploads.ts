// Upload queue for the file browser. Lives at module level so uploads keep
// going when the user switches tabs or folders.

import { useSyncExternalStore } from "react";
import { qs, uploadFile, type UploadHandle } from "../lib/api";
import { newClientId } from "../lib/format";
import type { FileEntry } from "../lib/types";

export interface UploadItem {
  id: string;
  name: string;
  size: number;
  loaded: number;
  dir: string;
  status: "queued" | "uploading" | "done" | "failed";
  error?: string;
  startedAt?: number;
  speed: number;
  file: File;
}

const MAX_PARALLEL = 2;
let items: UploadItem[] = [];
const handles = new Map<string, UploadHandle<FileEntry>>();
const listeners = new Set<() => void>();
const doneListeners = new Set<(dir: string) => void>();

function emit() {
  items = [...items];
  listeners.forEach((fn) => fn());
}

function patch(id: string, p: Partial<UploadItem>) {
  items = items.map((it) => (it.id === id ? { ...it, ...p } : it));
  listeners.forEach((fn) => fn());
}

function pump() {
  const running = items.filter((i) => i.status === "uploading").length;
  let slots = MAX_PARALLEL - running;
  for (const it of items) {
    if (slots <= 0) break;
    if (it.status !== "queued") continue;
    slots--;
    start(it);
  }
}

function start(it: UploadItem) {
  const startedAt = performance.now();
  patch(it.id, { status: "uploading", startedAt, loaded: 0, error: undefined });
  const handle = uploadFile<FileEntry>(`/api/files/upload${qs({ dir: it.dir, name: it.name })}`, it.file, (loaded) => {
    const secs = (performance.now() - startedAt) / 1000;
    patch(it.id, { loaded, speed: secs > 0.3 ? loaded / secs : 0 });
  });
  handles.set(it.id, handle);
  handle.promise
    .then(() => {
      patch(it.id, { status: "done", loaded: it.size });
      doneListeners.forEach((fn) => fn(it.dir));
    })
    .catch((e) => {
      if (e.status === -1) items = items.filter((x) => x.id !== it.id);
      else patch(it.id, { status: "failed", error: e.message });
      emit();
    })
    .finally(() => {
      handles.delete(it.id);
      pump();
    });
}

export const uploads = {
  add(files: File[], dir: string) {
    for (const file of files) {
      items.push({ id: newClientId(), name: file.name, size: file.size, loaded: 0, dir, status: "queued", speed: 0, file });
    }
    emit();
    pump();
  },
  cancel(id: string) {
    const h = handles.get(id);
    if (h) h.abort();
    else {
      items = items.filter((x) => x.id !== id);
      emit();
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
