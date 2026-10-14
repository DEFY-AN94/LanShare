import { memo, useState } from "react";
import { Icon } from "../components/Icon";
import { useToast } from "../components/Toast";
import { copyText } from "../lib/clipboard";
import { browserDownload, browserPreview, canShareFiles, hostOpen, hostOpenUrl, hostReveal } from "../lib/fileActions";
import { fileCategory, formatBytes, formatDayTime, isInlineImage, isPreviewable, splitLinks } from "../lib/format";
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
}

export const MessageBubble = memo(function MessageBubble({ message: m, mine, isHost, onMediaLoad, onShare }: BubbleProps) {
  const toast = useToast();
  const [imgFailed, setImgFailed] = useState(false);
  const title = formatDayTime(m.created_at);

  if (m.kind === "text") {
    const text = m.text ?? "";
    const longOrCode = text.includes("\n") && /[{};<>=]|^\s{2,}/m.test(text);
    return (
      <div className={`msg-row ${mine ? "mine" : "theirs"}`}>
        <div className={`bubble ${longOrCode ? "bubble-mono" : ""}`} title={title}>
          <LinkedText text={text} isHost={isHost} />
        </div>
        <CopyButton text={text} />
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
    if (isHost) return void run(hostOpen(file.path));
    if (isPreviewable(file.name)) browserPreview(`${url}?inline=1`);
    else browserDownload(url, file.name);
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
        {file.exists && (
          <div className="file-actions">
            {isHost ? (
              <button className="link-btn" onClick={() => void run(hostReveal(file.path))}>
                在文件夹中显示
              </button>
            ) : (
              <>
                <button className="link-btn" onClick={() => browserDownload(url, file.name)}>
                  下载
                </button>
                {canShareFiles() && /^(image|video)$/.test(fileCategory(file.name)) && (
                  <button className="link-btn" onClick={() => onShare(m)}>
                    存储到相册
                  </button>
                )}
              </>
            )}
          </div>
        )}
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
                {failed ? formatBytes(size) : `${formatBytes(item.loaded)} / ${formatBytes(size)}`}
              </span>
              {!failed && (
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
            <button className="link-btn text-danger" onClick={() => onRetry(item.client_id)} title={item.error}>
              发送失败，点按重试
            </button>
            <button className="icon-btn small" aria-label="删除" onClick={() => onDiscard(item.client_id)}>
              <Icon name="x" size={14} />
            </button>
          </>
        ) : (
          <>
            <span className="muted">{item.kind === "file" ? `${pct}%` : "发送中…"}</span>
            {item.kind === "file" && (
              <button className="icon-btn small" aria-label="取消" onClick={() => onDiscard(item.client_id)}>
                <Icon name="x" size={14} />
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
}
