// Conversation state kept at module level, so pending sends, uploads and
// retries survive switching tabs or conversations (the Chat view unmounts).

import { ApiError, api, qs, uploadFile, type UploadHandle } from "../lib/api";
import { events } from "../lib/events";
import { newClientId } from "../lib/format";
import type { Message, MessagePage } from "../lib/types";

export interface PendingMessage {
  client_id: string;
  kind: "text" | "file";
  text?: string;
  file?: File;
  loaded: number;
  status: "sending" | "failed";
  error?: string;
  /** False for errors a retry cannot fix (e.g. 400/403). */
  retryable?: boolean;
  created_at: number;
}

export interface ConversationState {
  messages: Message[];
  pending: PendingMessage[];
  hasMore: boolean;
  loading: boolean;
  loadingOlder: boolean;
  loaded: boolean;
}

const PAGE = 50;
const CATCH_UP_PAGE = 200;
const MAX_PARALLEL_UPLOADS = 2;

const EMPTY: ConversationState = { messages: [], pending: [], hasMore: false, loading: false, loadingOlder: false, loaded: false };
const convs = new Map<string, ConversationState>();
const deviceOf = new Map<string, string | null>();
const listeners = new Map<string, Set<() => void>>();
const handles = new Map<string, UploadHandle<Message>>();
let uploadQueue: { key: string; client_id: string }[] = [];
let errorHandler: ((msg: string) => void) | null = null;

export const keyFor = (deviceId: string | null) => deviceId ?? "self";

export function getConversation(key: string): ConversationState {
  return convs.get(key) ?? EMPTY;
}

export function subscribe(key: string, fn: () => void): () => void {
  let set = listeners.get(key);
  if (!set) {
    set = new Set();
    listeners.set(key, set);
  }
  set.add(fn);
  return () => set.delete(fn);
}

export function setErrorHandler(fn: ((msg: string) => void) | null) {
  errorHandler = fn;
}

function update(key: string, fn: (s: ConversationState) => ConversationState) {
  convs.set(key, fn(getConversation(key)));
  listeners.get(key)?.forEach((l) => l());
}

function merge(key: string, incoming: Message[]) {
  if (!incoming.length) return;
  const done = new Set(incoming.map((m) => m.client_id).filter(Boolean));
  update(key, (s) => {
    const byId = new Map(s.messages.map((m) => [m.id, m]));
    for (const m of incoming) byId.set(m.id, m);
    return {
      ...s,
      messages: [...byId.values()].sort((a, b) => a.id - b.id),
      pending: done.size ? s.pending.filter((p) => !done.has(p.client_id)) : s.pending,
    };
  });
}

function patchPending(key: string, client_id: string, patch: Partial<PendingMessage>) {
  update(key, (s) => ({ ...s, pending: s.pending.map((p) => (p.client_id === client_id ? { ...p, ...patch } : p)) }));
}

const isRetryable = (e: unknown) => {
  const status = (e as ApiError).status ?? 0;
  return status <= 0 || status === 408 || status === 429 || status >= 500;
};

function fail(key: string, client_id: string, e: unknown) {
  patchPending(key, client_id, { status: "failed", error: (e as Error).message, retryable: isRetryable(e) });
}

const listUrl = (key: string, params: Record<string, number | undefined>) =>
  `/api/messages${qs({ device: deviceOf.get(key) ?? undefined, ...params })}`;

/** First load of a conversation; later mounts only catch up. */
export async function ensureLoaded(key: string, deviceId: string | null) {
  ensureSubscribed();
  deviceOf.set(key, deviceId);
  const s = getConversation(key);
  if (s.loaded) return catchUp(key);
  if (s.loading) return;
  update(key, (st) => ({ ...st, loading: true }));
  try {
    const page = await api.get<MessagePage>(listUrl(key, { limit: PAGE }));
    // Merge, not replace: SSE may already have delivered newer messages.
    merge(key, page.messages);
    update(key, (st) => ({ ...st, hasMore: page.has_more, loaded: true }));
  } catch (e) {
    errorHandler?.((e as Error).message);
  } finally {
    update(key, (st) => ({ ...st, loading: false }));
  }
}

/** Pages forward from the newest known message, so nothing missed stays missing. */
async function catchUp(key: string) {
  const s = getConversation(key);
  if (!s.loaded) return;
  let after = s.messages[s.messages.length - 1]?.id;
  try {
    if (after === undefined) {
      const page = await api.get<MessagePage>(listUrl(key, { limit: PAGE }));
      merge(key, page.messages);
      return;
    }
    for (;;) {
      const page = await api.get<MessagePage>(listUrl(key, { after, limit: CATCH_UP_PAGE }));
      merge(key, page.messages);
      if (!page.has_more || !page.messages.length) break;
      after = page.messages[page.messages.length - 1].id;
    }
  } catch {
    // The next reconnect triggers another catch-up.
  }
}

export async function loadOlder(key: string) {
  const s = getConversation(key);
  if (s.loadingOlder || !s.hasMore || !s.messages.length) return;
  update(key, (st) => ({ ...st, loadingOlder: true }));
  try {
    const page = await api.get<MessagePage>(listUrl(key, { before: s.messages[0].id, limit: PAGE }));
    merge(key, page.messages);
    update(key, (st) => ({ ...st, hasMore: page.has_more }));
  } catch (e) {
    errorHandler?.((e as Error).message);
  } finally {
    update(key, (st) => ({ ...st, loadingOlder: false }));
  }
}

async function postText(key: string, client_id: string, text: string) {
  try {
    const m = await api.post<Message>("/api/messages", { device: deviceOf.get(key), client_id, text });
    merge(key, [m]);
  } catch (e) {
    fail(key, client_id, e);
  }
}

function pumpUploads() {
  while (handles.size < MAX_PARALLEL_UPLOADS && uploadQueue.length) {
    const { key, client_id } = uploadQueue.shift()!;
    const p = getConversation(key).pending.find((x) => x.client_id === client_id);
    if (!p?.file) continue;
    const handle = uploadFile<Message>(
      `/api/messages/upload${qs({ device: deviceOf.get(key) ?? undefined, client_id, name: p.file.name })}`,
      p.file,
      (loaded) => patchPending(key, client_id, { loaded }),
    );
    handles.set(client_id, handle);
    handle.promise
      .then((m) => merge(key, [m]))
      .catch((e) => {
        if (e.status === -1) update(key, (s) => ({ ...s, pending: s.pending.filter((x) => x.client_id !== client_id) }));
        else fail(key, client_id, e);
      })
      .finally(() => {
        handles.delete(client_id);
        pumpUploads();
      });
  }
}

export function sendText(key: string, text: string) {
  const client_id = newClientId();
  update(key, (s) => ({
    ...s,
    pending: [...s.pending, { client_id, kind: "text", text, loaded: 0, status: "sending", created_at: Date.now() }],
  }));
  void postText(key, client_id, text);
}

export function sendFiles(key: string, files: File[]) {
  const fresh: PendingMessage[] = files.map((file) => ({
    client_id: newClientId(),
    kind: "file",
    file,
    loaded: 0,
    status: "sending",
    created_at: Date.now(),
  }));
  update(key, (s) => ({ ...s, pending: [...s.pending, ...fresh] }));
  uploadQueue.push(...fresh.map((p) => ({ key, client_id: p.client_id })));
  pumpUploads();
}

export function retry(key: string, client_id: string) {
  const p = getConversation(key).pending.find((x) => x.client_id === client_id);
  if (!p) return;
  patchPending(key, client_id, { status: "sending", error: undefined, loaded: 0 });
  if (p.kind === "text" && p.text) void postText(key, client_id, p.text);
  else if (p.file) {
    uploadQueue.push({ key, client_id });
    pumpUploads();
  }
}

export function discard(key: string, client_id: string) {
  handles.get(client_id)?.abort();
  uploadQueue = uploadQueue.filter((q) => q.client_id !== client_id);
  update(key, (s) => ({ ...s, pending: s.pending.filter((p) => p.client_id !== client_id) }));
}

/** After reconnecting: fill gaps and re-send whatever failed for a transient reason. */
function onReconnect() {
  for (const key of convs.keys()) {
    void catchUp(key);
    for (const p of getConversation(key).pending) {
      if (p.status === "failed" && p.retryable !== false) retry(key, p.client_id);
    }
  }
}

let subscribed = false;
function ensureSubscribed() {
  if (subscribed) return;
  subscribed = true;
  events.on("msg", (data) => {
    const m = data as Message;
    // Host: one conversation per device. Device: its single conversation.
    const key = convs.has(m.device_id) ? m.device_id : convs.has("self") ? "self" : null;
    if (key) merge(key, [m]);
  });
  events.on("resync", onReconnect);
  window.addEventListener("online", onReconnect);
  events.on("cleared", () => {
    for (const key of convs.keys()) update(key, (s) => ({ ...s, messages: [], hasMore: false }));
  });
  // Automatic cleanup on the computer deleted everything older than `before`.
  events.on("pruned", (data) => {
    const before = (data as { before?: number } | null)?.before;
    if (typeof before !== "number") return;
    for (const key of convs.keys()) {
      update(key, (s) => {
        const kept = s.messages.filter((m) => m.created_at >= before);
        // Everything older is gone as well, so there is nothing more to page in.
        return kept.length === s.messages.length ? s : { ...s, messages: kept, hasMore: false };
      });
    }
  });
}
