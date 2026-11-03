import { memo, useRef, useState, type MouseEvent } from "react";
import { Icon } from "../components/Icon";
import { useToast } from "../components/Toast";
import { copyText } from "../lib/clipboard";
import { canShareFiles, hostOpenMessageFile, hostOpenUrl, hostRevealMessageFile } from "../lib/fileActions";
import { fileCategory, formatBytes, formatDayTime, isInlineImage, splitLinks } from "../lib/format";
import { coarsePointer } from "../lib/pointer";
import type { Message } from "../lib/types";
import type { PendingMessage } from "./useConversation";

const CATEGORY_ICON: Record<string, string> = {
  image: "image",
  video: "video",
  audio: "music",
  pdf: "pdf",
  text: "text",
  archive: "archive",
  doc: "doc",
  other: "file",
};

export function fileIcon(name: string) {
  return CATEGORY_ICON[fileCategory(name)];
}

function LinkedText({ text, isHost }: { text: string; isHost: boolean }) {
  return (
    <>
      {splitLinks(text).map((part, i) =>
        part.href ? (
          <a
            key={i}
            href={part.href}
            target="_blank"
            rel="noopener noreferrer"
            onClick={(e) => {
              // The desktop window must never navigate away; open in the system browser.
              if (isHost) {
                e.preventDefault();
                void hostOpenUrl(part.href!);
              }
            }}
          >
            {part.text}
          </a>
        ) : (
          <span key={i}>{part.text}</span>
        ),
      )}
    </>
  );
}

export function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const toast = useToast();
  return (
    <button
      className={`copy-btn ${copied ? "is-copied" : ""}`}
      aria-label="复制"
      title="复制"
      onClick={async () => {
        const ok = await copyText(text);
        if (ok) {
          setCopied(true);
          toast("已复制", "success");
          window.setTimeout(() => setCopied(false), 1500);
        } else {
          toast("复制失败，请长按文字手动复制", "error");
        }
      }}
    >
      <Icon name={copied ? "check" : "copy"} size={16} />
    </button>
  );
}

interface BubbleProps {
  message: Message;
  mine: boolean;
  isHost: boolean;
  onMediaLoad: () => void;
  onShare: (m: Message) => void;
  /** A phone opening the attachment: its preview, or a download when `download` is set or it has none. */
  onOpenFile: (m: Message, download: boolean) => void;
  /** Opens the message's actions (copy, delete). */
  onActions: (m: Message) => void;
}

// On the iPhone a tap on the bubble opens its actions, but not a tap on a link
// or one that ends a text selection. With a mouse a click only selects (a
// double-click picks a word), so the actions are on the ⋯ button and on
// right-click; a right-click on a link, or on a bubble with text selected in
// it, keeps the system menu.
const hasSelection = () => !!window.getSelection()?.toString();
const onLink = (e: MouseEvent) => !!(e.target as HTMLElement).closest("a");

export const MessageBubble = memo(function MessageBubble({ message: m, mine, isHost, onMediaLoad, onShare, onOpenFile, onActions }: BubbleProps) {
  const toast = useToast();
  const [imgFailed, setImgFailed] = useState(false);
  // Read when the button goes down: a Mac selects the word under a right-click
  // before the menu event arrives.
  const selectedBefore = useRef(false);
  const title = formatDayTime(m.created_at);

  if (m.kind === "text") {
    const text = m.text ?? "";
    const longOrCode = text.includes("\n") && /[{};<>=]|^\s{2,}/m.test(text);
    return (
      <div className={`msg-row ${mine ? "mine" : "theirs"}`}>
        <div
          className={`bubble ${longOrCode ? "bubble-mono" : ""}`}
          title={title}
          onClick={(e) => coarsePointer() && !onLink(e) && !hasSelection() && onActions(m)}
          onPointerDown={(e) => {
            // Only a selection in this bubble keeps the system menu, whose Copy copies the selection.
            const sel = window.getSelection();
            selectedBefore.current = !!sel?.rangeCount && !!sel.toString() && sel.getRangeAt(0).intersectsNode(e.currentTarget);
          }}
          onContextMenu={(e) => {
            if (coarsePointer() || onLink(e) || selectedBefore.current) return;
            e.preventDefault();
            window.getSelection()?.removeAllRanges();
            onActions(m);
          }}
        >
          <LinkedText text={text} isHost={isHost} />
        </div>
        <CopyButton text={text} />
        <button className="copy-btn msg-more" aria-label="更多操作" title="更多操作" onClick={() => onActions(m)}>
          <Icon name="more" size={16} />
        </button>
      </div>
    );
  }

  const file = m.file;
  if (!file) return null;
  const url = `/api/messages/${m.id}/file`;
  const showImage = file.exists && isInlineImage(file.name) && !imgFailed;
  const run = (p: Promise<unknown>) => p.catch((e) => toast((e as Error).message, "error"));

  const open = () => {
    if (!file.exists) return toast("文件已被移动或删除", "error");
    if (isHost) return void run(hostOpenMessageFile(m.id, (msg) => toast(msg)));
    onOpenFile(m, false);
  };

  return (
    <div className={`msg-row ${mine ? "mine" : "theirs"}`}>
      <div className="file-bubble" title={title}>
        {showImage ? (
          <button className="bubble-image" onClick={open} aria-label={`查看 ${file.name}`}>
            <img src={`${url}?inline=1`} alt={file.name} loading="lazy" onLoad={onMediaLoad} onError={() => setImgFailed(true)} />
          </button>
        ) : (
          <button className="file-card" onClick={open}>
            <span className="file-card-icon">
              <Icon name={fileIcon(file.name)} size={22} />
            </span>
            <span className="file-card-body">
              <span className="file-card-name">{file.name}</span>
              <span className="file-card-meta">{file.exists ? formatBytes(file.size) : "文件已被移动或删除"}</span>
            </span>
          </button>
        )}
        <div className="file-actions">
          {file.exists &&
            (isHost ? (
              <button className="link-btn" onClick={() => void run(hostRevealMessageFile(m.id))}>
                在文件夹中显示
              </button>
            ) : (
              <>
                <button className="link-btn" onClick={() => onOpenFile(m, true)}>
                  下载
                </button>
                {canShareFiles() && /^(image|video)$/.test(fileCategory(file.name)) && (
                  <button className="link-btn" onClick={() => onShare(m)}>
                    存储到相册
                  </button>
                )}
              </>
            ))}
          <button className="link-btn link-muted" onClick={() => onActions(m)}>
            删除
          </button>
        </div>
      </div>
    </div>
  );
});

interface PendingProps {
  item: PendingMessage;
  onRetry: (id: string) => void;
  onDiscard: (id: string) => void;
}

export function PendingBubble({ item, onRetry, onDiscard }: PendingProps) {
  const failed = item.status === "failed";
  const waiting = item.status === "waiting";
  const size = item.file?.size ?? 0;
  const pct = size ? Math.min(100, Math.round((item.loaded / size) * 100)) : 0;
  return (
    <div className="msg-row mine pending">
      {item.kind === "text" ? (
        <div className="bubble">{item.text}</div>
      ) : (
        <div className="file-bubble">
          <div className="file-card">
            <span className="file-card-icon">
              <Icon name={fileIcon(item.file?.name ?? "")} size={22} />
            </span>
            <span className="file-card-body">
              <span className="file-card-name">{item.file?.name}</span>
              <span className="file-card-meta">
                {failed || waiting ? formatBytes(size) : `${formatBytes(item.loaded)} / ${formatBytes(size)}`}
              </span>
              {!failed && !waiting && (
                <span className="progress">
                  <span style={{ width: `${pct}%` }} />
                </span>
              )}
            </span>
          </div>
        </div>
      )}
      <div className="pending-status">
        {failed ? (
          <>
            <button className="pending-action text-danger" onClick={() => onRetry(item.client_id)} title={item.error}>
              <Icon name="refresh" size={14} />{" "}
              {item.retryable === false ? `发送失败：${item.error ?? ""}` : item.error ? `发送失败：${item.error}（点按重试）` : "发送失败，点按重试"}
            </button>
            <button className="pending-action" aria-label="删除" onClick={() => onDiscard(item.client_id)}>
              <Icon name="x" size={16} />
            </button>
          </>
        ) : waiting ? (
          <>
            <span className="muted">等待连接…</span>
            <button className="pending-action" aria-label="删除" onClick={() => onDiscard(item.client_id)}>
              <Icon name="x" size={16} />
            </button>
          </>
        ) : (
          <>
            <span className="muted">{item.kind === "file" ? `${pct}%` : "发送中…"}</span>
            {item.kind === "file" && (
              <button className="pending-action" aria-label="取消" onClick={() => onDiscard(item.client_id)}>
                <Icon name="x" size={16} />
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
}
