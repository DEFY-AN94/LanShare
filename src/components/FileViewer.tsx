// Opening and downloading files on the phone. In a Safari tab a preview opens
// in a new tab and a download uses Safari's own download page, as before. The
// Home Screen app has no browser controls, so both would strand the user with
// no way back: there LanShare shows images, video, audio and text in its own
// viewer and saves files through the share sheet. Checked in the iOS Simulator
// (iOS 27): in-page media plays and seeks with the cookie, and the share sheet
// returns to the same page after saving or cancelling.

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { browserDownload, browserPreview, canShareFiles, fetchTextStart, inlineUrl, isStandalone, TEXT_LIMIT } from "../lib/fileActions";
import { decodeText, formatBytes, isPreviewable, viewKind, type ViewKind } from "../lib/format";
import { ShareSheet } from "./ShareSheet";

export interface FileTarget {
  /** The download URL, without `inline`. */
  url: string;
  name: string;
  size: number;
  mime?: string;
}

export interface FileOpener {
  /** Whether the file has a preview of its own (in the Home Screen app, one in LanShare's viewer). */
  canPreview: (name: string) => boolean;
  /** A tap on the file: its preview if it has one, else a download. */
  open: (t: FileTarget) => void;
  preview: (t: FileTarget) => void;
  download: (t: FileTarget) => void;
  /** The viewer and the save sheet, to render once. */
  element: ReactNode;
}

export function useFileOpener(): FileOpener {
  const standalone = useMemo(isStandalone, []);
  const [viewing, setViewing] = useState<{ target: FileTarget; kind: ViewKind } | null>(null);
  const [saving, setSaving] = useState<FileTarget | null>(null);

  const download = useCallback(
    (t: FileTarget) => {
      if (standalone && canShareFiles()) setSaving(t);
      else browserDownload(t.url, t.name);
    },
    [standalone],
  );
  const preview = useCallback(
    (t: FileTarget) => {
      if (!standalone) return browserPreview(inlineUrl(t.url));
      const kind = viewKind(t.name);
      if (kind) setViewing({ target: t, kind });
      else download(t);
    },
    [standalone, download],
  );
  const open = useCallback((t: FileTarget) => (isPreviewable(t.name) ? preview(t) : download(t)), [preview, download]);
  const canPreview = useCallback((name: string) => (standalone ? viewKind(name) !== null : isPreviewable(name)), [standalone]);
  const closeViewer = useCallback(() => setViewing(null), []);

  const element = (
    <>
      {viewing && <FileViewer target={viewing.target} kind={viewing.kind} onSave={() => download(viewing.target)} onClose={closeViewer} />}
      {saving && (
        <ShareSheet url={saving.url} name={saving.name} mime={saving.mime ?? ""} size={saving.size} purpose="files" onClose={() => setSaving(null)} />
      )}
    </>
  );
  return { canPreview, open, preview, download, element };
}

interface ViewerProps {
  target: FileTarget;
  kind: ViewKind;
  onSave: () => void;
  onClose: () => void;
}

function FileViewer({ target, kind, onSave, onClose }: ViewerProps) {
  const media = useRef<HTMLMediaElement | null>(null);
  const [failed, setFailed] = useState(false);
  const src = inlineUrl(target.url);

  // Closing stops playback and the download behind it.
  useEffect(() => {
    const m = media.current;
    return () => {
      if (!m) return;
      m.pause();
      m.removeAttribute("src");
      m.load();
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const fail = () => setFailed(true);
  const keep = (m: HTMLMediaElement | null) => {
    if (m) media.current = m;
  };
  let body: ReactNode;
  if (kind === "text") body = <TextView url={target.url} size={target.size} />;
  else if (failed) body = <div className="viewer-note">{kind === "image" ? "图片无法显示" : "无法播放，iPhone 可能不支持这个格式"}。可以下载到「文件」后用别的 App 打开。</div>;
  else if (kind === "image") body = <img src={src} alt={target.name} onError={fail} />;
  else if (kind === "video") body = <video ref={keep} src={src} controls playsInline preload="metadata" onError={fail} />;
  else body = <audio ref={keep} src={src} controls preload="metadata" onError={fail} />;

  return (
    <div className="viewer" role="dialog" aria-modal="true" aria-label={target.name}>
      <div className="viewer-bar">
        <span className="viewer-title ellipsis">{target.name}</span>
        <button className="btn btn-small" onClick={onClose}>
          关闭
        </button>
      </div>
      <div className={`viewer-body ${kind === "text" ? "is-text" : ""}`}>{body}</div>
      <div className="viewer-foot">
        <button className="btn btn-primary" onClick={onSave}>
          下载到「文件」
        </button>
      </div>
    </div>
  );
}

function TextView({ url, size }: { url: string; size: number }) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const cut = size > TEXT_LIMIT;

  useEffect(() => {
    const abort = new AbortController();
    fetchTextStart(url, abort.signal)
      .then((bytes) => !abort.signal.aborted && setText(decodeText(bytes, cut)))
      .catch((e) => !abort.signal.aborted && setError((e as Error).message));
    return () => abort.abort();
  }, [url, cut]);

  if (error) return <div className="viewer-note text-danger">{error}</div>;
  if (text === null) return <div className="viewer-note">正在从电脑读取…</div>;
  return (
    <>
      {cut && (
        <div className="viewer-cut">
          只显示前 {formatBytes(TEXT_LIMIT)}（共 {formatBytes(size)}），完整内容请下载到「文件」
        </div>
      )}
      <pre className="viewer-text">{text}</pre>
    </>
  );
}
