import { Fragment, useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ActionSheet, type SheetAction } from "../components/ActionSheet";
import { useDialog } from "../components/Dialog";
import { ShareSheet } from "../components/ShareSheet";
import { useToast } from "../components/Toast";
import { copyText } from "../lib/clipboard";
import { formatDayTime } from "../lib/format";
import type { Message } from "../lib/types";
import { Composer } from "./Composer";
import { MessageBubble, PendingBubble } from "./MessageBubble";
import { deleteMessage, keyFor } from "./store";
import { useConversation } from "./useConversation";

interface Props {
  /** Conversation id on the host; null on a paired device. */
  deviceId: string | null;
  isHost: boolean;
  disabled?: boolean;
  /** Why the message box is off, in place of "已断开连接". */
  disabledText?: string;
  /** Not connected right now: messages queue until the connection is back. */
  offline?: boolean;
  /** May also move a file message's attachment to the computer's Trash. */
  canTrashFiles?: boolean;
  header?: ReactNode;
  emptyHint?: string;
}

const GROUP_GAP_MS = 5 * 60_000;

export function Chat({ deviceId, isHost, disabled, disabledText, offline, canTrashFiles = isHost, header, emptyHint }: Props) {
  const conv = useConversation(deviceId);
  const toast = useToast();
  const [actionFor, setActionFor] = useState<Message | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);
  const restore = useRef<{ height: number; top: number; firstId: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const [sharing, setSharing] = useState<Message | null>(null);
  const dialog = useDialog();

  useEffect(() => {
    stickToBottom.current = true;
  }, [deviceId]);

  const scrollToBottom = useCallback(() => {
    const el = listRef.current;
    if (el && stickToBottom.current) el.scrollTop = el.scrollHeight;
  }, []);

  const total = conv.messages.length + conv.pending.length;
  const lastKey = conv.pending.length ? conv.pending[conv.pending.length - 1].client_id : conv.messages[conv.messages.length - 1]?.id;
  const firstId = conv.messages[0]?.id;

  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const r = restore.current;
    if (r && firstId !== r.firstId) {
      // Older messages were prepended: keep what the user was looking at in place.
      el.scrollTop = r.top + (el.scrollHeight - r.height);
      restore.current = null;
      return;
    }
    if (r && !conv.loadingOlder) restore.current = null; // load ended without new messages
    scrollToBottom();
  }, [total, lastKey, firstId, conv.loadingOlder, scrollToBottom]);

  // Stay pinned to the newest message when the list gets shorter
  // (keyboard opening on iPhone, composer growing).
  useEffect(() => {
    const el = listRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => scrollToBottom());
    ro.observe(el);
    return () => ro.disconnect();
  }, [scrollToBottom]);

  const onScroll = () => {
    const el = listRef.current;
    if (!el) return;
    stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    if (el.scrollTop < 80 && conv.hasMore && !conv.loadingOlder && firstId !== undefined) {
      restore.current = { height: el.scrollHeight, top: el.scrollTop, firstId };
      void conv.loadOlder();
    }
  };

  const discard = async (id: string) => {
    const p = conv.pending.find((x) => x.client_id === id);
    if (p?.kind === "text" && p.status !== "sending") {
      const ok = await dialog.confirm({ title: "删除这条未发送的消息？", message: p.text, confirmText: "删除", danger: true });
      if (!ok) return;
    }
    conv.discard(id);
  };

  const sendText = (text: string) => {
    stickToBottom.current = true;
    conv.sendText(text);
  };
  const sendFiles = (files: File[]) => {
    stickToBottom.current = true;
    conv.sendFiles(files);
  };

  const mine = (m: Message) => (isHost ? m.from_host : !m.from_host);

  const remove = async (m: Message, trashFile: boolean) => {
    const ok = await dialog.confirm({
      title: trashFile ? "删除这条消息和文件？" : "删除这条消息？",
      message: trashFile
        ? "电脑和手机上的这条消息都会删除，无法恢复；文件会被移到电脑的回收站，可以从回收站恢复。"
        : `电脑和手机上的这条消息都会删除，无法恢复。${m.kind === "file" ? "文件仍保留在电脑上。" : ""}`,
      confirmText: "删除",
      danger: true,
    });
    if (!ok) return;
    try {
      await deleteMessage(m.id, trashFile);
    } catch (e) {
      toast((e as Error).message, "error");
    }
  };

  const actionsFor = (m: Message): SheetAction[] => {
    const list: SheetAction[] = [];
    if (m.kind === "text") {
      list.push({
        label: "复制",
        icon: "copy",
        onSelect: () => void copyText(m.text ?? "").then((ok) => toast(ok ? "已复制" : "复制失败，请长按文字手动复制", ok ? "success" : "error")),
      });
      list.push({ label: "删除", icon: "trash", danger: true, onSelect: () => void remove(m, false) });
    } else {
      list.push({ label: "删除消息，保留文件", icon: "trash", danger: true, onSelect: () => void remove(m, false) });
      if (canTrashFiles && m.file?.exists) {
        list.push({ label: "删除消息，并把文件移到电脑的回收站", icon: "trash", danger: true, onSelect: () => void remove(m, true) });
      }
    }
    return list;
  };

  return (
    <div
      className={`chat ${dragging ? "is-dragging" : ""}`}
      onDragOver={(e) => {
        if (disabled || !e.dataTransfer.types.includes("Files")) return;
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={(e) => {
        if (e.currentTarget.contains(e.relatedTarget as Node)) return;
        setDragging(false);
      }}
      onDrop={(e) => {
        e.preventDefault();
        setDragging(false);
        const files = Array.from(e.dataTransfer.files);
        if (files.length && !disabled) sendFiles(files);
      }}
    >
      {header}
      <div className="chat-list" ref={listRef} onScroll={onScroll}>
        {conv.loadingOlder && <div className="chat-hint">正在加载更早的消息…</div>}
        {!conv.loading && total === 0 && <div className="chat-empty">{emptyHint ?? "还没有消息，发一条试试"}</div>}
        {conv.messages.map((m, i) => {
          const prev = conv.messages[i - 1];
          const showTime = !prev || m.created_at - prev.created_at > GROUP_GAP_MS;
          return (
            <Fragment key={m.id}>
              {showTime && <div className="chat-time">{formatDayTime(m.created_at)}</div>}
              <MessageBubble message={m} mine={mine(m)} isHost={isHost} onMediaLoad={scrollToBottom} onShare={setSharing} onActions={setActionFor} />
            </Fragment>
          );
        })}
        {conv.pending.map((p) => (
          <PendingBubble key={p.client_id} item={p} onRetry={conv.retry} onDiscard={(id) => void discard(id)} />
        ))}
      </div>
      <Composer
        disabled={disabled}
        disabledText={disabledText}
        offline={offline}
        draftKey={keyFor(deviceId)}
        persistDraft={!isHost}
        onSendText={sendText}
        onSendFiles={sendFiles}
      />
      {dragging && <div className="drop-overlay">松开即可发送文件</div>}
      {actionFor && <ActionSheet actions={actionsFor(actionFor)} onClose={() => setActionFor(null)} />}
      {sharing?.file && (
        <ShareSheet
          url={`/api/messages/${sharing.id}/file`}
          name={sharing.file.name}
          mime={sharing.file.mime}
          size={sharing.file.size}
          onClose={() => setSharing(null)}
        />
      )}
    </div>
  );
}
