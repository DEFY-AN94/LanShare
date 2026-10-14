// "存储到相册" on iPhone: fetch the file first, then call navigator.share from
// a fresh tap (iOS rejects share() once the original tap has expired).

import { useEffect, useState } from "react";
import { fetchAsFile } from "../lib/fileActions";
import { formatBytes } from "../lib/format";
import { useToast } from "./Toast";

interface Props {
  url: string;
  name: string;
  mime: string;
  size: number;
  onClose: () => void;
}

export function ShareSheet({ url, name, mime, size, onClose }: Props) {
  const [file, setFile] = useState<File | null>(null);
  const [error, setError] = useState<string | null>(null);
  const toast = useToast();

  useEffect(() => {
    let cancelled = false;
    fetchAsFile(url, name, mime)
      .then((f) => !cancelled && setFile(f))
      .catch((e) => !cancelled && setError((e as Error).message));
    return () => {
      cancelled = true;
    };
  }, [url, name, mime]);

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

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal dialog" role="dialog" aria-modal="true">
        <h2 className="dialog-title">存储到相册 / 分享</h2>
        <div className="dialog-message">
          <div className="ellipsis">{name}</div>
          <div className="muted">{formatBytes(size)}</div>
          {!file && !error && <div className="muted">正在从电脑读取…</div>}
          {error && <div className="text-danger">{error}</div>}
          {file && <div className="muted">已准备好，点「打开分享」后选择「存储图像」或「存储视频」。</div>}
        </div>
        <div className="dialog-actions">
          <button className="btn" onClick={onClose}>
            取消
          </button>
          <button className="btn btn-primary" disabled={!file} onClick={share}>
            打开分享
          </button>
        </div>
      </div>
    </div>
  );
}
