import { useCallback, useEffect, useRef, useState } from "react";
import { ActionSheet, type SheetAction } from "../components/ActionSheet";
import { useDialog } from "../components/Dialog";
import { Icon } from "../components/Icon";
import { ShareSheet } from "../components/ShareSheet";
import { useToast } from "../components/Toast";
import { fileIcon } from "../chat/MessageBubble";
import { api, qs } from "../lib/api";
import { events } from "../lib/events";
import { browserDownload, browserPreview, canShareFiles, hostOpen, hostReveal } from "../lib/fileActions";
import { fileCategory, formatBytes, formatShort, formatSpeed, isPreviewable } from "../lib/format";
import type { DirListing, FileEntry } from "../lib/types";
import { uploads, useUploads } from "./uploads";

interface Props {
  path: string;
  isHost: boolean;
  disabled?: boolean;
  onNavigate: (path: string) => void;
}

const downloadUrl = (path: string, inline = false) => `/api/files/download${qs({ path, inline: inline ? 1 : undefined })}`;

export function FileBrowser({ path, isHost, disabled, onNavigate }: Props) {
  const [listing, setListing] = useState<DirListing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [menuFor, setMenuFor] = useState<FileEntry | null>(null);
  const [sharing, setSharing] = useState<FileEntry | null>(null);
  const [dragging, setDragging] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const toast = useToast();
  const dialog = useDialog();
  const pathRef = useRef(path);
  pathRef.current = path;

  const load = useCallback(
    async (quiet = false) => {
      if (!quiet) setLoading(true);
      try {
        const data = await api.get<DirListing>(`/api/files${qs({ path })}`);
        if (pathRef.current !== path) return;
        setListing(data);
        setError(null);
      } catch (e) {
        if (pathRef.current !== path) return;
        setError((e as Error).message);
        setListing(null);
      } finally {
        if (pathRef.current === path) setLoading(false);
      }
    },
    [path],
  );

  useEffect(() => {
    void load();
    const offFiles = events.on("files", (data) => {
      const dir = (data as { dir?: string })?.dir ?? "";
      if (dir === pathRef.current) void load(true);
    });
    const offResync = events.on("resync", () => void load(true));
    const offDone = uploads.onDone((dir) => dir === pathRef.current && void load(true));
    return () => {
      offFiles();
      offResync();
      offDone();
    };
  }, [load]);

  const canWrite = !!listing?.can_write && !disabled;
  const segments = path ? path.split("/") : [];
  const run = async (p: Promise<unknown>, okText?: string) => {
    try {
      await p;
      if (okText) toast(okText, "success");
    } catch (e) {
      toast((e as Error).message, "error");
    }
  };

  const openEntry = (e: FileEntry) => {
    if (e.is_dir) return onNavigate(e.path);
    if (isHost) return void run(hostOpen(e.path, (m) => toast(m)));
    if (isPreviewable(e.name)) browserPreview(downloadUrl(e.path, true));
    else browserDownload(downloadUrl(e.path), e.name);
  };

  const rename = async (e: FileEntry) => {
    const name = await dialog.prompt({ title: "重命名", initial: e.name, confirmText: "保存", selectStem: !e.is_dir });
    if (!name || name === e.name) return;
    await run(api.post("/api/files/rename", { path: e.path, name }));
    void load(true);
  };

  const remove = async (e: FileEntry) => {
    const ok = await dialog.confirm({
      title: `删除「${e.name}」？`,
      message: e.is_dir ? "整个文件夹会被移到电脑的回收站，可以从回收站恢复。" : "文件会被移到电脑的回收站，可以从回收站恢复。",
      confirmText: "删除",
      danger: true,
    });
    if (!ok) return;
    await run(api.post("/api/files/delete", { path: e.path }), "已移到回收站");
    void load(true);
  };

  const mkdir = async () => {
    const name = await dialog.prompt({ title: "新建文件夹", placeholder: "文件夹名称", confirmText: "创建" });
    if (!name) return;
    await run(api.post("/api/files/mkdir", { dir: path, name }));
    void load(true);
  };

  const actionsFor = (e: FileEntry): SheetAction[] => {
    const list: SheetAction[] = [];
    if (e.is_dir) {
      list.push({ label: "打开", icon: "folder", onSelect: () => onNavigate(e.path) });
    } else if (isHost) {
      list.push({ label: "用默认程序打开", icon: "external", onSelect: () => void run(hostOpen(e.path, (m) => toast(m))) });
    } else {
      if (isPreviewable(e.name)) list.push({ label: "预览", icon: "external", onSelect: () => browserPreview(downloadUrl(e.path, true)) });
      list.push({ label: "下载到「文件」", icon: "download", onSelect: () => browserDownload(downloadUrl(e.path), e.name) });
      if (canShareFiles() && /^(image|video)$/.test(fileCategory(e.name))) {
        list.push({ label: "存储到相册 / 分享", icon: "image", onSelect: () => setSharing(e) });
      }
    }
    if (isHost) list.push({ label: "在文件夹中显示", icon: "reveal", onSelect: () => void run(hostReveal(e.path)) });
    if (canWrite) {
      list.push({ label: "重命名", icon: "edit", onSelect: () => void rename(e) });
      list.push({ label: "删除", icon: "trash", danger: true, onSelect: () => void remove(e) });
    }
    return list;
  };

  return (
    <div
      className={`files ${dragging ? "is-dragging" : ""}`}
      onDragOver={(e) => {
        if (!canWrite || !e.dataTransfer.types.includes("Files")) return;
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
        if (files.length && canWrite) uploads.add(files, path);
      }}
    >
      <div className="files-toolbar">
        <nav className="crumbs" aria-label="路径">
          <button className="crumb" onClick={() => onNavigate("")}>
            共享文件夹
          </button>
          {segments.map((seg, i) => (
            <span key={i} className="crumb-wrap">
              <Icon name="chevron-right" size={14} className="crumb-sep" />
              <button className="crumb" onClick={() => onNavigate(segments.slice(0, i + 1).join("/"))}>
                {seg}
              </button>
            </span>
          ))}
        </nav>
        <div className="toolbar-actions">
          {isHost && (
            <button className="icon-btn" title="在文件夹中打开" aria-label="在文件夹中打开" onClick={() => void run(hostOpen(path))}>
              <Icon name="reveal" />
            </button>
          )}
          <button className="icon-btn" title="刷新" aria-label="刷新" onClick={() => void load()}>
            <Icon name="refresh" />
          </button>
          {canWrite && (
            <>
              <button className="icon-btn" title="新建文件夹" aria-label="新建文件夹" onClick={() => void mkdir()}>
                <Icon name="folder-plus" />
              </button>
              <button className="btn btn-primary btn-small" onClick={() => fileInput.current?.click()}>
                <Icon name="upload" size={16} />
                <span>上传</span>
              </button>
            </>
          )}
          <input
            ref={fileInput}
            type="file"
            multiple
            hidden
            onChange={(e) => {
              const files = Array.from(e.target.files ?? []);
              e.target.value = "";
              if (files.length) uploads.add(files, path);
            }}
          />
        </div>
      </div>

      <div className="files-list">
        {loading && !listing && <div className="list-hint">正在读取…</div>}
        {error && (
          <div className="list-hint">
            <p className="text-danger">{error}</p>
            {path && (
              <button className="btn" onClick={() => onNavigate("")}>
                回到共享文件夹
              </button>
            )}
          </div>
        )}
        {listing && listing.entries.length === 0 && (
          <div className="list-hint">{canWrite ? "这个文件夹是空的，点「上传」添加文件" : "这个文件夹是空的"}</div>
        )}
        {listing?.entries.map((e) => (
          <div key={e.path} className="file-row">
            <button className="file-row-main" onClick={() => openEntry(e)}>
              <span className={`file-row-icon ${e.is_dir ? "is-dir" : ""}`}>
                <Icon name={e.is_dir ? "folder" : fileIcon(e.name)} size={22} />
              </span>
              <span className="file-row-body">
                <span className="file-row-name">{e.name}</span>
                <span className="file-row-meta">
                  {e.is_dir ? "文件夹" : formatBytes(e.size)} · {formatShort(e.modified)}
                </span>
              </span>
            </button>
            <button className="icon-btn" aria-label={`${e.name} 的更多操作`} onClick={() => setMenuFor(e)}>
              <Icon name="more" />
            </button>
          </div>
        ))}
      </div>

      <UploadPanel />
      {dragging && <div className="drop-overlay">松开即可上传到「{segments[segments.length - 1] ?? "共享文件夹"}」</div>}
      {menuFor && (
        <ActionSheet
          title={menuFor.name}
          subtitle={menuFor.is_dir ? "文件夹" : `${formatBytes(menuFor.size)} · ${formatShort(menuFor.modified)}`}
          actions={actionsFor(menuFor)}
          onClose={() => setMenuFor(null)}
        />
      )}
      {sharing && (
        <ShareSheet
          url={downloadUrl(sharing.path)}
          name={sharing.name}
          mime=""
          size={sharing.size}
          onClose={() => setSharing(null)}
        />
      )}
    </div>
  );
}

function UploadPanel() {
  const items = useUploads();
  if (!items.length) return null;
  const active = items.filter((i) => i.status === "uploading" || i.status === "queued").length;
  return (
    <div className="upload-panel">
      <div className="upload-panel-head">
        <span>{active ? `正在上传 ${active} 个文件` : "上传完成"}</span>
        {active === 0 && (
          <button className="link-btn" onClick={() => uploads.clearFinished()}>
            清除
          </button>
        )}
      </div>
      <div className="upload-items">
        {items.map((it) => {
          const pct = it.size ? Math.round((it.loaded / it.size) * 100) : it.status === "done" ? 100 : 0;
          return (
            <div key={it.id} className={`upload-item is-${it.status}`}>
              <div className="upload-item-top">
                <span className="ellipsis">{it.name}</span>
                <span className="muted nowrap">
                  {it.status === "uploading" && `${pct}% · ${formatSpeed(it.speed)}`}
                  {it.status === "queued" && "等待中"}
                  {it.status === "done" && "完成"}
                  {it.status === "failed" && <span className="text-danger">失败</span>}
                </span>
                {it.status === "failed" ? (
                  <button className="link-btn" onClick={() => uploads.retry(it.id)} title={it.error}>
                    重试
                  </button>
                ) : (
                  it.status !== "done" && (
                    <button className="icon-btn small" aria-label="取消上传" onClick={() => uploads.cancel(it.id)}>
                      <Icon name="x" size={14} />
                    </button>
                  )
                )}
              </div>
              {(it.status === "uploading" || it.status === "queued") && (
                <span className="progress">
                  <span style={{ width: `${pct}%` }} />
                </span>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
