import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ActionSheet, type SheetAction } from "../components/ActionSheet";
import { useDialog } from "../components/Dialog";
import { Icon } from "../components/Icon";
import { usePhotoTip } from "../components/PhotoTip";
import { useFileOpener, type FileTarget } from "../components/FileViewer";
import { ShareSheet } from "../components/ShareSheet";
import { useToast } from "../components/Toast";
import { fileIcon } from "../chat/MessageBubble";
import { api, qs } from "../lib/api";
import { events } from "../lib/events";
import { canShareFiles, hostOpen, hostReveal } from "../lib/fileActions";
import { fileCategory, formatBytes, formatShort, formatSpeed } from "../lib/format";
import { loadLocal, saveLocal } from "../lib/storage";
import type { DirListing, FileEntry } from "../lib/types";
import { useDropZone } from "../lib/useDropZone";
import { openPicker } from "../lib/wakeLock";
import { uploads, useUploads } from "./uploads";

type SortBy = "name" | "modified" | "size";
const SORT_KEY = "files.sort";
const SORT_LABELS: Record<SortBy, string> = { name: "按名称", modified: "最近修改", size: "按大小" };

/** Folders first, then by the chosen order: name A–Z, newest first, or largest first. */
function sortEntries(entries: FileEntry[], by: SortBy): FileEntry[] {
  const byName = (a: FileEntry, b: FileEntry) => a.name.localeCompare(b.name, "zh-Hans-CN", { numeric: true, sensitivity: "base" });
  return [...entries].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    if (by === "modified") return b.modified - a.modified || byName(a, b);
    if (by === "size" && !a.is_dir) return b.size - a.size || byName(a, b);
    return byName(a, b);
  });
}

interface Props {
  path: string;
  isHost: boolean;
  disabled?: boolean;
  onNavigate: (path: string) => void;
}

const downloadUrl = (path: string) => `/api/files/download${qs({ path })}`;
const target = (e: FileEntry): FileTarget => ({ url: downloadUrl(e.path), name: e.name, size: e.size });

export function FileBrowser({ path, isHost, disabled, onNavigate }: Props) {
  const [listing, setListing] = useState<DirListing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [menuFor, setMenuFor] = useState<FileEntry | null>(null);
  const [sharing, setSharing] = useState<FileEntry | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const photoTip = usePhotoTip(() => openPicker(fileInput.current));
  const opener = useFileOpener();
  const toast = useToast();
  const dialog = useDialog();
  const pathRef = useRef(path);
  pathRef.current = path;
  const [sortBy, setSortBy] = useState<SortBy>(() => {
    const saved = loadLocal<string>(SORT_KEY, "name");
    return saved === "modified" || saved === "size" ? saved : "name";
  });
  const [filter, setFilter] = useState("");
  const [filterInput, setFilterInput] = useState("");
  // Pinyin and other input methods: filter only once the word is finished.
  const composing = useRef(false);
  // A filter belongs to one folder.
  useEffect(() => {
    setFilter("");
    setFilterInput("");
  }, [path]);
  const changeSort = (by: SortBy) => {
    setSortBy(by);
    saveLocal(SORT_KEY, by);
  };
  const shown = useMemo(() => {
    if (!listing) return [];
    const q = filter.trim().toLowerCase();
    return sortEntries(q ? listing.entries.filter((e) => e.name.toLowerCase().includes(q)) : listing.entries, sortBy);
  }, [listing, filter, sortBy]);

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
  const drop = useDropZone(canWrite, (files) => uploads.add(files, path));
  const { dragging } = drop;
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
    opener.open(target(e));
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
      if (opener.canPreview(e.name)) list.push({ label: "预览", icon: "external", onSelect: () => opener.preview(target(e)) });
      list.push({ label: "下载到「文件」", icon: "download", onSelect: () => opener.download(target(e)) });
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
      {...drop.handlers}
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
              <button className="btn btn-primary btn-small" onClick={photoTip.open}>
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

      {listing && listing.entries.length > 0 && (
        <div className="files-filter">
          <input
            className="input"
            type="search"
            value={filterInput}
            placeholder="按名称筛选"
            aria-label="按名称筛选"
            onChange={(e) => {
              setFilterInput(e.target.value);
              if (!composing.current) setFilter(e.target.value);
            }}
            onCompositionStart={() => (composing.current = true)}
            onCompositionEnd={(e) => {
              composing.current = false;
              setFilter(e.currentTarget.value);
            }}
          />
          <label>
            <span className="sr-only">排序</span>
            <select className="input" value={sortBy} onChange={(e) => changeSort(e.target.value as SortBy)}>
              {(Object.keys(SORT_LABELS) as SortBy[]).map((k) => (
                <option key={k} value={k}>
                  {SORT_LABELS[k]}
                </option>
              ))}
            </select>
          </label>
        </div>
      )}

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
        {listing && listing.entries.length > 0 && shown.length === 0 && <div className="list-hint">没有名称里包含「{filter.trim()}」的项目</div>}
        {shown.map((e) => (
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
      {photoTip.element}
      {opener.element}
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
  const failedCount = items.filter((i) => i.status === "failed").length;
  return (
    <div className="upload-panel">
      <div className="upload-panel-head">
        <span>{active ? `正在上传 ${active} 个文件` : failedCount ? `有 ${failedCount} 个文件没有上传成功` : "上传完成"}</span>
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
                  it.retryable !== false && (
                    <button className="link-btn" onClick={() => uploads.retry(it.id)}>
                      重试
                    </button>
                  )
                ) : (
                  it.status !== "done" && (
                    <button className="icon-btn small" aria-label="取消上传" onClick={() => uploads.cancel(it.id)}>
                      <Icon name="x" size={14} />
                    </button>
                  )
                )}
              </div>
              {it.status === "failed" && it.error && <div className="upload-item-error text-danger small">{it.error}</div>}
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
