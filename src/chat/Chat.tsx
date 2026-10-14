import { Fragment, useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ShareSheet } from "../components/ShareSheet";
import { formatDayTime } from "../lib/format";
import type { Message } from "../lib/types";
import { Composer } from "./Composer";
import { MessageBubble, PendingBubble } from "./MessageBubble";
import { useConversation } from "./useConversation";

interface Props {
  /** Conversation id on the host; null on a paired device. */
  deviceId: string | null;
  isHost: boolean;
  disabled?: boolean;
  header?: ReactNode;
  emptyHint?: string;
}

const GROUP_GAP_MS = 5 * 60_000;

export function Chat({ deviceId, isHost, disabled, header, emptyHint }: Props) {
  const conv = useConversation(deviceId);
  const listRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);
  const restore = useRef<{ height: number; top: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const [sharing, setSharing] = useState<Message | null>(null);

  useEffect(() => {
    stickToBottom.current = true;
  }, [deviceId]);

  const scrollToBottom = useCallback(() => {
    const el = listRef.current;
    if (el && stickToBottom.current) el.scrollTop = el.scrollHeight;
  }, []);

  const total = conv.messages.length + conv.pending.length;
  const lastKey = conv.pending.length ? conv.pending[conv.pending.length - 1].client_id : conv.messages[conv.messages.length - 1]?.id;

  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el) return;
    if (restore.current) {
      // Older messages were prepended: keep what the user was looking at in place.
      el.scrollTop = restore.current.top + (el.scrollHeight - restore.current.height);
      restore.current = null;
      return;
    }
    scrollToBottom();
  }, [total, lastKey, scrollToBottom]);

  const onScroll = () => {
    const el = listRef.current;
    if (!el) return;
    stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    if (el.scrollTop < 80 && conv.hasMore && !conv.loadingOlder) {
      restore.current = { height: el.scrollHeight, top: el.scrollTop };
      void conv.loadOlder();
    }
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
              <MessageBubble message={m} mine={mine(m)} isHost={isHost} onMediaLoad={scrollToBottom} onShare={setSharing} />
            </Fragment>
          );
        })}
        {conv.pending.map((p) => (
          <PendingBubble key={p.client_id} item={p} onRetry={conv.retry} onDiscard={conv.discard} />
        ))}
      </div>
      <Composer disabled={disabled} onSendText={sendText} onSendFiles={sendFiles} />
      {dragging && <div className="drop-overlay">松开即可发送文件</div>}
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
