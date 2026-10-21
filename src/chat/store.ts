// Conversation state kept at module level, so pending sends, uploads and
// retries survive switching tabs or conversations (the Chat view unmounts).
// On the phone, unsent texts are also kept in localStorage (the "outbox"),
// so they survive iOS reloading the page and are sent once connected.

import { ApiError, api, qs, uploadFile, type UploadHandle } from "../lib/api";
import { events } from "../lib/events";
import { newClientId } from "../lib/format";
import { conversationCleared, hasLocal, loadLocalAll, onRemovedElsewhere, removeLocal, saveLocal } from "../lib/storage";
import type { Message, MessagePage } from "../lib/types";
import { holdScreen } from "../lib/wakeLock";

export interface PendingMessage {
  client_id: string;
  kind: "text" | "file";
  text?: string;
  file?: File;
  loaded: number;
  /** waiting: not connected right now; sent once the connection is back. */
  status: "sending" | "waiting" | "failed";
  error?: string;
  /** False for errors a retry cannot fix (e.g. 400/403). */
  retryable?: boolean;
  /** Quick retries used while the connection looked fine; reset on reconnect. */
  quickRetries?: number;
  /** Queued so long ago that it goes out only on 重试 (see OUTBOX_AUTO_SEND_MS). */
  expired?: boolean;
  /** The user chose to send it anyway: it is no longer held back for its age. */
  confirmed?: boolean;
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
/** A text that has not gone through by then waits for the connection instead. */
const SEND_TIMEOUT_MS = 8_000;
/** A send that failed while the connection looked fine is tried again after this long, */
const RETRY_SOON_MS = 2_000;
/** this many times at most; after that it shows as failed, with 重试. */
const MAX_QUICK_RETRIES = 3;
/** "Would it fit?" asked before an attachment is sent. */
const CHECK_TIMEOUT_MS = 8_000;
/** The phone's own conversation; the only one whose outbox is kept. */
const SELF = "self";
/** One stored entry per unsent text, so several tabs never overwrite each other's. */
const OUTBOX_PREFIX = "outbox.";
/**
 * An unsent text older than this is no longer sent by itself, only on 重试:
 * the computer forgets deleted messages after 180 days (DELETIONS_KEPT_MS in
 * core/state.rs), and a month-old text should not turn up unannounced.
 */
const OUTBOX_AUTO_SEND_MS = 30 * 86_400_000;

interface OutboxItem {
  client_id: string;
  text: string;
  created_at: number;
  confirmed?: boolean;
}

const EMPTY: ConversationState = { messages: [], pending: [], hasMore: false, loading: false, loadingOlder: false, loaded: false };
const convs = new Map<string, ConversationState>();
const deviceOf = new Map<string, string | null>();
const listeners = new Map<string, Set<() => void>>();
const handles = new Map<string, UploadHandle<Message>>();
/** Uploads asking the computer whether they fit; they count toward the parallel limit. */
const starting = new Set<string>();
let uploadQueue: { key: string; client_id: string }[] = [];
let errorHandler: ((msg: string) => void) | null = null;
let releaseScreen: (() => void) | null = null;
/** The outbox entries this page has stored, by client id. */
const stored = new Map<string, string>();

export const keyFor = (deviceId: string | null) => deviceId ?? SELF;

const connected = () => events.state === "open";
/** Unpaired, or this phone paired again in another tab: queued items belong to the old pairing. */
const stopped = () => conversationCleared();

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
  const next = fn(getConversation(key));
  convs.set(key, next);
  if (key === SELF) saveOutbox(next.pending);
  listeners.get(key)?.forEach((l) => l());
}

/** Texts that still have to reach the computer; files cannot be kept. */
function saveOutbox(pending: PendingMessage[]) {
  const keep = new Set<string>();
  for (const p of pending) {
    if (p.kind !== "text" || !p.text || (p.status === "failed" && p.retryable === false && !p.expired)) continue;
    keep.add(p.client_id);
    const item: OutboxItem = { client_id: p.client_id, text: p.text, created_at: p.created_at, ...(p.confirmed ? { confirmed: true } : {}) };
    const json = JSON.stringify(item);
    if (stored.get(p.client_id) === json) continue;
    if (saveLocal(OUTBOX_PREFIX + p.client_id, item)) stored.set(p.client_id, json);
  }
  // Sent or dropped here. Entries another tab stored are left alone.
  for (const cid of [...stored.keys()]) {
    if (keep.has(cid)) continue;
    stored.delete(cid);
    removeLocal(OUTBOX_PREFIX + cid);
  }
}

/** Also picks up what other tabs left; a text stored twice is still sent once (same client id). */
function restoreOutbox() {
  const known = new Set(getConversation(SELF).pending.map((p) => p.client_id));
  const restored: PendingMessage[] = [];
  for (const i of loadLocalAll<OutboxItem>(OUTBOX_PREFIX)) {
    if (!i || typeof i.text !== "string" || typeof i.client_id !== "string" || known.has(i.client_id)) continue;
    known.add(i.client_id);
    const created_at = typeof i.created_at === "number" ? i.created_at : Date.now();
    const confirmed = i.confirmed === true;
    const base = { client_id: i.client_id, kind: "text" as const, text: i.text, loaded: 0, created_at, confirmed };
    restored.push(tooOld(base) ? { ...base, ...EXPIRED } : { ...base, status: "waiting" });
  }
  restored.sort((a, b) => a.created_at - b.created_at);
  if (restored.length) update(SELF, (s) => ({ ...s, pending: [...s.pending, ...restored] }));
}

/** A phone text that is no longer sent by itself (see OUTBOX_AUTO_SEND_MS). */
const tooOld = (p: Pick<PendingMessage, "created_at" | "confirmed">) => !p.confirmed && Date.now() - p.created_at > OUTBOX_AUTO_SEND_MS;

/** Kept until the user sends it with 重试 or deletes it. */
const EXPIRED: Partial<PendingMessage> & { status: "failed" } = { status: "failed", retryable: false, expired: true, error: "超过 30 天没有发出，点按发送" };

/** Another tab sent or deleted this queued text: this tab no longer sends it. */
function droppedElsewhere(client_id: string) {
  stored.delete(client_id);
  update(SELF, (s) => ({ ...s, pending: s.pending.filter((p) => p.client_id !== client_id) }));
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

function removeMessages(ids: number[]) {
  if (!ids.length) return;
  const gone = new Set(ids);
  for (const key of convs.keys()) {
    const s = getConversation(key);
    if (s.messages.some((m) => gone.has(m.id))) update(key, (st) => ({ ...st, messages: st.messages.filter((m) => !gone.has(m.id)) }));
  }
}

function patchPending(key: string, client_id: string, patch: Partial<PendingMessage>) {
  update(key, (s) => ({ ...s, pending: s.pending.map((p) => (p.client_id === client_id ? { ...p, ...patch } : p)) }));
}

const isRetryable = (e: unknown) => {
  const status = (e as ApiError).status ?? 0;
  return status <= 0 || status === 408 || status === 429 || status >= 500;
};

function fail(key: string, client_id: string, e: unknown) {
  const status = (e as ApiError).status;
  // An earlier attempt went through and was deleted since: nothing left to send.
  if (status === 410) {
    update(key, (s) => ({ ...s, pending: s.pending.filter((p) => p.client_id !== client_id) }));
    return;
  }
  // Not reachable: wait for the connection instead of showing an error.
  if (status === 0 && !connected()) {
    patchPending(key, client_id, { status: "waiting", error: undefined, loaded: 0 });
    return;
  }
  // The connection still looks open (a stalled upload, a busy computer): try again soon,
  if (status === 0) {
    const quick = getConversation(key).pending.find((p) => p.client_id === client_id)?.quickRetries ?? 0;
    if (quick < MAX_QUICK_RETRIES) {
      patchPending(key, client_id, { status: "waiting", error: undefined, loaded: 0, quickRetries: quick + 1 });
      setTimeout(flush, RETRY_SOON_MS);
      return;
    }
  }
  // then offer 重试 rather than wait for a reconnect that may never come.
  const error = status === 0 ? undefined : (e as Error).message;
  patchPending(key, client_id, { status: "failed", error, retryable: isRetryable(e), loaded: 0 });
}

const listUrl = (key: string, params: Record<string, number | undefined>) =>
  `/api/messages${qs({ device: deviceOf.get(key) ?? undefined, ...params })}`;

/** First load of a conversation; later mounts only catch up. */
export async function ensureLoaded(key: string, deviceId: string | null) {
  ensureSubscribed();
  deviceOf.set(key, deviceId);
  if (key === SELF && !getConversation(key).loaded && !getConversation(key).loading) restoreOutbox();
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
  flush();
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
  if (stopped()) return;
  // Sent or deleted in another tab while this one slept through the storage event.
  if (key === SELF && stored.has(client_id) && hasLocal(OUTBOX_PREFIX + client_id) === false) return droppedElsewhere(client_id);
  if (!connected()) {
    patchPending(key, client_id, { status: "waiting" });
    return;
  }
  patchPending(key, client_id, { status: "sending", error: undefined });
  try {
    // The same client_id on every attempt: the computer keeps one copy.
    const m = await api.post<Message>("/api/messages", { device: deviceOf.get(key), client_id, text }, SEND_TIMEOUT_MS);
    merge(key, [m]);
  } catch (e) {
    fail(key, client_id, e);
  }
}

function updateScreenHold() {
  const active = handles.size + starting.size > 0;
  if (active && !releaseScreen) releaseScreen = holdScreen();
  else if (!active && releaseScreen) {
    releaseScreen();
    releaseScreen = null;
  }
}

function pumpUploads() {
  while (handles.size + starting.size < MAX_PARALLEL_UPLOADS && uploadQueue.length) {
    if (!connected() || stopped()) break;
    const { key, client_id } = uploadQueue.shift()!;
    const p = getConversation(key).pending.find((x) => x.client_id === client_id);
    if (!p?.file) continue;
    void startUpload(key, client_id, p.file);
  }
  // Files that wait for the connection say so.
  for (const q of uploadQueue) {
    const p = getConversation(q.key).pending.find((x) => x.client_id === q.client_id);
    if (p && p.status === "sending" && !handles.has(q.client_id) && !connected()) patchPending(q.key, q.client_id, { status: "waiting" });
  }
}

async function startUpload(key: string, client_id: string, file: File) {
  starting.add(client_id);
  updateScreenHold();
  patchPending(key, client_id, { status: "sending", error: undefined, loaded: 0 });
  try {
    // Ask first: a refusal for disk space then arrives as a clear message
    // instead of a connection the computer had to cut.
    await api.get(`/api/upload-check${qs({ size: file.size })}`, CHECK_TIMEOUT_MS);
  } catch (e) {
    starting.delete(client_id);
    updateScreenHold();
    fail(key, client_id, e);
    pumpUploads();
    return;
  }
  starting.delete(client_id);
  if (!getConversation(key).pending.some((x) => x.client_id === client_id)) {
    // Discarded while asking.
    updateScreenHold();
    pumpUploads();
    return;
  }
  {
    const handle = uploadFile<Message>(
      `/api/messages/upload${qs({ device: deviceOf.get(key) ?? undefined, client_id, name: file.name, size: file.size })}`,
      file,
      (loaded) => patchPending(key, client_id, { loaded }),
    );
    handles.set(client_id, handle);
    updateScreenHold();
    handle.promise
      .then((m) => merge(key, [m]))
      .catch((e) => {
        if (e.status === -1) update(key, (s) => ({ ...s, pending: s.pending.filter((x) => x.client_id !== client_id) }));
        else fail(key, client_id, e);
      })
      .finally(() => {
        handles.delete(client_id);
        updateScreenHold();
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
    status: connected() ? "sending" : "waiting",
    created_at: Date.now(),
  }));
  update(key, (s) => ({ ...s, pending: [...s.pending, ...fresh] }));
  uploadQueue.push(...fresh.map((p) => ({ key, client_id: p.client_id })));
  pumpUploads();
}

export function retry(key: string, client_id: string) {
  const p = getConversation(key).pending.find((x) => x.client_id === client_id);
  if (!p) return;
  // Only the user retries an expired text: from then on it goes out like any other.
  const confirm = p.expired ? { expired: false, confirmed: true, retryable: undefined } : {};
  patchPending(key, client_id, { status: "sending", error: undefined, loaded: 0, ...confirm });
  if (p.kind === "text" && p.text) void postText(key, client_id, p.text);
  else if (p.file) {
    if (!uploadQueue.some((q) => q.client_id === client_id) && !handles.has(client_id) && !starting.has(client_id)) uploadQueue.push({ key, client_id });
    pumpUploads();
  }
}

export function discard(key: string, client_id: string) {
  handles.get(client_id)?.abort();
  uploadQueue = uploadQueue.filter((q) => q.client_id !== client_id);
  update(key, (s) => ({ ...s, pending: s.pending.filter((p) => p.client_id !== client_id) }));
}

/** Phone: texts an earlier visit left unsent go out once connected, even if the chat is never opened. */
export function startOutbox() {
  ensureSubscribed();
  restoreOutbox();
  flush();
}

/** Deletes a sent message on both sides; `trashFile` also moves its attachment to the Trash. */
export async function deleteMessage(id: number, trashFile = false) {
  await api.del(`/api/messages/${id}${qs({ trash_file: trashFile ? 1 : undefined })}`);
  removeMessages([id]);
}

/** Sends everything that was waiting for the connection. */
function flush() {
  if (!connected() || stopped()) return;
  for (const key of convs.keys()) {
    for (const p of getConversation(key).pending) {
      if (p.status !== "waiting" && (p.status !== "failed" || p.retryable === false)) continue;
      if (key === SELF && p.kind === "text" && tooOld(p)) patchPending(key, p.client_id, EXPIRED);
      else retry(key, p.client_id);
    }
  }
  pumpUploads();
}

/** After reconnecting: fill gaps, then send what waited (with fresh quick retries). */
function onReconnect() {
  for (const key of convs.keys()) {
    void catchUp(key);
    if (getConversation(key).pending.some((p) => p.quickRetries)) {
      update(key, (s) => ({ ...s, pending: s.pending.map((p) => (p.quickRetries ? { ...p, quickRetries: 0 } : p)) }));
    }
  }
  flush();
}

let subscribed = false;
function ensureSubscribed() {
  if (subscribed) return;
  subscribed = true;
  events.on("msg", (data) => {
    const m = data as Message;
    // Host: one conversation per device. Device: its single conversation.
    const key = convs.has(m.device_id) ? m.device_id : convs.has(SELF) ? SELF : null;
    if (key) merge(key, [m]);
  });
  events.on("msg_deleted", (data) => {
    const ids = (data as { ids?: unknown } | null)?.ids;
    if (Array.isArray(ids)) removeMessages(ids.filter((x): x is number => typeof x === "number"));
  });
  events.on("resync", onReconnect);
  onRemovedElsewhere(OUTBOX_PREFIX, (client_id) => {
    const p = getConversation(SELF).pending.find((x) => x.client_id === client_id);
    // One already on its way here finishes; the computer keeps a single copy.
    if (p && p.status !== "sending") droppedElsewhere(client_id);
  });
  // The first connection too: texts typed before it opened go out now.
  events.onState((s) => s === "open" && flush());
  window.addEventListener("online", onReconnect);
  // The computer cleared every chat (up to message id `through`), or its
  // automatic cleanup deleted everything older than `before` (ms).
  const dropOld = (before: number, through: number) => {
    for (const key of convs.keys()) {
      update(key, (s) => {
        const kept = s.messages.filter((m) => m.created_at >= before && m.id > through);
        // Everything older is gone as well, so there is nothing more to page in.
        return kept.length === s.messages.length ? s : { ...s, messages: kept, hasMore: false };
      });
    }
  };
  const num = (v: unknown) => (typeof v === "number" ? v : 0);
  events.on("cleared", (data) => {
    const through = (data as { through?: unknown } | null)?.through;
    dropOld(0, typeof through === "number" ? through : Number.MAX_SAFE_INTEGER);
  });
  events.on("pruned", (data) => {
    const d = (data ?? {}) as { before?: unknown; through?: unknown };
    dropOld(num(d.before), num(d.through));
  });
}
