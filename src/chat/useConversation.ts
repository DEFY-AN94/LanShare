import { useCallback, useEffect, useRef, useState } from "react";
import { api, qs, uploadFile, type UploadHandle } from "../lib/api";
import { events } from "../lib/events";
import { newClientId } from "../lib/format";
import type { Message, MessagePage } from "../lib/types";
import { useToast } from "../components/Toast";

export interface PendingMessage {
  client_id: string;
  kind: "text" | "file";
  text?: string;
  file?: File;
  loaded: number;
  status: "sending" | "failed";
  error?: string;
  created_at: number;
}

const PAGE = 50;
const MAX_PARALLEL_UPLOADS = 2;

/**
 * Messages of one conversation. `deviceId` is the conversation on the host;
 * a paired device passes null (it only has its own conversation).
 */
export function useConversation(deviceId: string | null) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [pending, setPending] = useState<PendingMessage[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const toast = useToast();
  const uploads = useRef(new Map<string, UploadHandle<Message>>());
  const queue = useRef<string[]>([]);
  const pendingRef = useRef<PendingMessage[]>([]);
  pendingRef.current = pending;

  const merge = useCallback((incoming: Message[]) => {
    if (!incoming.length) return;
    setMessages((prev) => {
      const byId = new Map(prev.map((m) => [m.id, m]));
      for (const m of incoming) byId.set(m.id, m);
      return [...byId.values()].sort((a, b) => a.id - b.id);
    });
    const done = new Set(incoming.map((m) => m.client_id).filter(Boolean));
    if (done.size) setPending((prev) => prev.filter((p) => !done.has(p.client_id)));
  }, []);

  const patchPending = useCallback((client_id: string, patch: Partial<PendingMessage>) => {
    setPending((prev) => prev.map((p) => (p.client_id === client_id ? { ...p, ...patch } : p)));
  }, []);

  const sendTextNow = useCallback(
    async (client_id: string, text: string) => {
      try {
        const m = await api.post<Message>("/api/messages", { device: deviceId, client_id, text });
        merge([m]);
      } catch (e) {
        patchPending(client_id, { status: "failed", error: (e as Error).message });
      }
    },
    [deviceId, merge, patchPending],
  );

  const pumpUploads = useCallback(() => {
    while (uploads.current.size < MAX_PARALLEL_UPLOADS && queue.current.length) {
      const client_id = queue.current.shift()!;
      const p = pendingRef.current.find((x) => x.client_id === client_id);
      if (!p?.file) continue;
      const handle = uploadFile<Message>(
        `/api/messages/upload${qs({ device: deviceId, client_id, name: p.file.name })}`,
        p.file,
        (loaded) => patchPending(client_id, { loaded }),
      );
      uploads.current.set(client_id, handle);
      handle.promise
        .then((m) => merge([m]))
        .catch((e) => {
          if (e.status === -1) setPending((prev) => prev.filter((x) => x.client_id !== client_id));
          else patchPending(client_id, { status: "failed", error: e.message });
        })
        .finally(() => {
          uploads.current.delete(client_id);
          pumpUploads();
        });
    }
  }, [deviceId, merge, patchPending]);

  const refreshLatest = useCallback(async () => {
    try {
      const page = await api.get<MessagePage>(`/api/messages${qs({ device: deviceId, limit: PAGE })}`);
      merge(page.messages);
    } catch {
      // the stream will trigger another resync when it reconnects
    }
  }, [deviceId, merge]);

  useEffect(() => {
    let cancelled = false;
    setMessages([]);
    setPending([]);
    setHasMore(false);
    setLoading(true);
    api
      .get<MessagePage>(`/api/messages${qs({ device: deviceId, limit: PAGE })}`)
      .then((page) => {
        if (cancelled) return;
        setMessages(page.messages);
        setHasMore(page.has_more);
      })
      .catch((e) => !cancelled && toast(e.message, "error"))
      .finally(() => !cancelled && setLoading(false));

    const offMsg = events.on("msg", (data) => {
      const m = data as Message;
      if (deviceId === null || m.device_id === deviceId) merge([m]);
    });
    const offResync = events.on("resync", () => {
      void refreshLatest();
      // Text that failed while offline goes out again automatically.
      for (const p of pendingRef.current) {
        if (p.kind === "text" && p.status === "failed" && p.text) {
          patchPending(p.client_id, { status: "sending", error: undefined });
          void sendTextNow(p.client_id, p.text);
        }
      }
    });
    const offCleared = events.on("cleared", () => {
      setMessages([]);
      setHasMore(false);
    });
    return () => {
      cancelled = true;
      offMsg();
      offResync();
      offCleared();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deviceId]);

  const loadOlder = useCallback(async () => {
    if (loadingOlder || !hasMore || !messages.length) return;
    setLoadingOlder(true);
    try {
      const page = await api.get<MessagePage>(`/api/messages${qs({ device: deviceId, before: messages[0].id, limit: PAGE })}`);
      merge(page.messages);
      setHasMore(page.has_more);
    } catch (e) {
      toast((e as Error).message, "error");
    } finally {
      setLoadingOlder(false);
    }
  }, [deviceId, hasMore, loadingOlder, merge, messages, toast]);

  const sendText = useCallback(
    (text: string) => {
      const client_id = newClientId();
      setPending((prev) => [...prev, { client_id, kind: "text", text, loaded: 0, status: "sending", created_at: Date.now() }]);
      void sendTextNow(client_id, text);
    },
    [sendTextNow],
  );

  const sendFiles = useCallback(
    (files: File[]) => {
      const fresh: PendingMessage[] = files.map((file) => ({
        client_id: newClientId(),
        kind: "file",
        file,
        loaded: 0,
        status: "sending",
        created_at: Date.now(),
      }));
      pendingRef.current = [...pendingRef.current, ...fresh];
      setPending((prev) => [...prev, ...fresh]);
      queue.current.push(...fresh.map((p) => p.client_id));
      pumpUploads();
    },
    [pumpUploads],
  );

  const retry = useCallback(
    (client_id: string) => {
      const p = pendingRef.current.find((x) => x.client_id === client_id);
      if (!p) return;
      patchPending(client_id, { status: "sending", error: undefined, loaded: 0 });
      if (p.kind === "text" && p.text) void sendTextNow(client_id, p.text);
      else if (p.file) {
        queue.current.push(client_id);
        pumpUploads();
      }
    },
    [patchPending, pumpUploads, sendTextNow],
  );

  const discard = useCallback((client_id: string) => {
    uploads.current.get(client_id)?.abort();
    queue.current = queue.current.filter((id) => id !== client_id);
    setPending((prev) => prev.filter((p) => p.client_id !== client_id));
  }, []);

  return { messages, pending, hasMore, loading, loadingOlder, loadOlder, sendText, sendFiles, retry, discard };
}
