// "存储到相册" on iPhone, and "下载到「文件」" in the Home Screen app: fetch the
// file first, then call navigator.share from a fresh tap (iOS rejects share()
// once the original tap has expired, about 3 s after it).

import { useEffect, useState } from "react";
import { copyText } from "../lib/clipboard";
import { fetchAsFile, SHARE_LIMIT } from "../lib/fileActions";
import { formatBytes } from "../lib/format";
import { useToast } from "./Toast";

interface Props {
  url: string;
  name: string;
  mime: string;
  size: number;
  /** "files": a download to Files from the Home Screen app, which has no download page to come back from. */
  purpose?: "photos" | "files";
  onClose: () => void;
}

export function ShareSheet({ url, name, mime, size, purpose = "photos", onClose }: Props) {
  const [file, setFile] = useState<File | null>(null);
  const [error, setError] = useState<string | null>(null);
  const toast = useToast();
  const tooLarge = purpose === "files" && size > SHARE_LIMIT;

  useEffect(() => {
    if (tooLarge) return;
    // Closing stops the transfer, so a large file is not read for nothing.
    const abort = new AbortController();
    fetchAsFile(url, name, mime, abort.signal)
      .then((f) => !abort.signal.aborted && setFile(f))
      .catch((e) => !abort.signal.aborted && setError((e as Error).message));
    return () => abort.abort();
  }, [url, name, mime, tooLarge]);

  const share = async () => {
    if (!file) return;
    try {
      if (navigator.canShare && !navigator.canShare({ files: [file] })) {
        toast("这个文件类型不支持系统分享", "error");
        return;
      }
      await navigator.share({ files: [file] });
      onClose();
    } catch (e) {
      if ((e as Error).name !== "AbortError") toast("分享失败", "error");
    }
  };

  const copyAddress = async () => {
    if (await copyText(window.location.origin)) toast("已复制地址", "success");
    else toast("复制失败", "error");
  };

  const ready = purpose === "files" ? "已准备好，点「打开分享」后选择「保存到“文件”」。" : "已准备好，点「打开分享」后选择「保存图像」或「保存视频」。";

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal dialog" role="dialog" aria-modal="true">
        <h2 className="dialog-title">{purpose === "files" ? "下载到「文件」" : "存储到相册 / 分享"}</h2>
        <div className="dialog-message">
          <div className="ellipsis">{name}</div>
          <div className="muted">{formatBytes(size)}</div>
          {tooLarge ? (
            <div>
              主屏幕上的 LanShare 只能保存 {formatBytes(SHARE_LIMIT)} 以内的文件。请在 Safari 里打开 {window.location.origin} 下载。
            </div>
          ) : (
            <>
              {!file && !error && <div className="muted">正在从电脑读取…</div>}
              {error && <div className="text-danger">{error}</div>}
              {file && <div className="muted">{ready}</div>}
            </>
          )}
        </div>
        <div className="dialog-actions">
          <button className="btn" onClick={onClose}>
            {tooLarge ? "关闭" : "取消"}
          </button>
          {tooLarge ? (
            <button className="btn btn-primary" onClick={() => void copyAddress()}>
              复制地址
            </button>
          ) : (
            <button className="btn btn-primary" disabled={!file} onClick={share}>
              打开分享
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
