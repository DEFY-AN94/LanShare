// Formatting helpers (Chinese UI).

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "—";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}

export function formatSpeed(bytesPerSec: number): string {
  return `${formatBytes(Math.max(0, Math.round(bytesPerSec)))}/s`;
}

const pad = (n: number) => String(n).padStart(2, "0");

export function formatClock(ts: number): string {
  const d = new Date(ts);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** "今天 14:32" / "昨天 09:10" / "10月12日 18:00" / "2025年3月1日 08:00" */
export function formatDayTime(ts: number): string {
  const d = new Date(ts);
  const now = new Date();
  const diffDays = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
  const clock = formatClock(ts);
  if (diffDays === 0) return `今天 ${clock}`;
  if (diffDays === 1) return `昨天 ${clock}`;
  if (d.getFullYear() === now.getFullYear()) return `${d.getMonth() + 1}月${d.getDate()}日 ${clock}`;
  return `${d.getFullYear()}年${d.getMonth() + 1}月${d.getDate()}日 ${clock}`;
}

/** Compact time for lists: "14:32", "昨天", "10/12". */
export function formatShort(ts: number): string {
  const d = new Date(ts);
  const now = new Date();
  const diffDays = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
  if (diffDays === 0) return formatClock(ts);
  if (diffDays === 1) return "昨天";
  if (d.getFullYear() === now.getFullYear()) return `${d.getMonth() + 1}/${d.getDate()}`;
  return `${d.getFullYear()}/${d.getMonth() + 1}/${d.getDate()}`;
}

export function formatRelative(ts: number | null): string {
  if (!ts) return "从未";
  const diff = Date.now() - ts;
  if (diff < 60_000) return "刚刚";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  return formatDayTime(ts);
}

export interface TextPart {
  text: string;
  href?: string;
}

const URL_RE = /https?:\/\/[^\s<>"'，。；！？、）】」』]+/gi;
const TRAILING = ".,;:!?)]}'\"";

/** Splits text into plain runs and http(s) links. Nothing else becomes a link. */
export function splitLinks(text: string): TextPart[] {
  const parts: TextPart[] = [];
  let last = 0;
  for (const m of text.matchAll(URL_RE)) {
    // A loop, not /[...]+$/: that regex retries a long punctuation run from every position.
    let end = m[0].length;
    while (end > 0 && TRAILING.includes(m[0][end - 1])) end--;
    const url = m[0].slice(0, end);
    const start = m.index ?? 0;
    if (start > last) parts.push({ text: text.slice(last, start) });
    parts.push({ text: url, href: url });
    last = start + url.length;
  }
  if (last < text.length) parts.push({ text: text.slice(last) });
  return parts;
}

export function guessDeviceName(): string {
  const ua = navigator.userAgent;
  if (/iPad/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)) return "iPad";
  if (/iPhone/.test(ua)) return "iPhone";
  if (/Android/.test(ua)) return /Mobile/.test(ua) ? "安卓手机" : "安卓平板";
  if (/Windows/.test(ua)) return "Windows 电脑";
  if (/Macintosh/.test(ua)) return "Mac";
  return "我的设备";
}

export type FileCategory = "image" | "video" | "audio" | "pdf" | "text" | "archive" | "doc" | "other";

const EXT: Record<string, FileCategory> = {};
for (const e of ["jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "avif", "svg", "tif", "tiff"]) EXT[e] = "image";
for (const e of ["mp4", "mov", "m4v", "webm", "mkv", "avi", "3gp"]) EXT[e] = "video";
for (const e of ["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus"]) EXT[e] = "audio";
EXT.pdf = "pdf";
for (const e of ["txt", "md", "log", "csv", "json", "xml", "yaml", "yml", "ini", "conf", "js", "ts", "py", "rs", "go", "java", "c", "cpp", "h", "sh", "css", "sql"]) EXT[e] = "text";
for (const e of ["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "dmg", "iso"]) EXT[e] = "archive";
for (const e of ["doc", "docx", "xls", "xlsx", "ppt", "pptx", "pages", "numbers", "key", "rtf", "odt"]) EXT[e] = "doc";

export function fileCategory(name: string): FileCategory {
  const i = name.lastIndexOf(".");
  if (i < 0) return "other";
  return EXT[name.slice(i + 1).toLowerCase()] ?? "other";
}

/** Types the browser can show inline that the server is willing to serve inline. */
export function isPreviewable(name: string): boolean {
  const c = fileCategory(name);
  // SVG can carry scripts, so the server only ever serves it as a download.
  if (c === "image") return !/\.(svg|tiff?)$/i.test(name);
  return c === "video" || c === "audio" || c === "pdf" || c === "text";
}

export type ViewKind = "image" | "video" | "audio" | "text";

/** What the Home Screen app's own viewer shows; anything else (a PDF too) is saved to Files instead. */
export function viewKind(name: string): ViewKind | null {
  if (!isPreviewable(name)) return null;
  const c = fileCategory(name);
  return c === "image" || c === "video" || c === "audio" || c === "text" ? c : null;
}

/**
 * Text from either computer: UTF-8, UTF-16 with a byte order mark, or else
 * GB18030 (Chinese text saved on Windows). The server labels all text as
 * UTF-8, so the bytes decide. `cut` says the bytes stop mid-file, where a
 * character may be split; it is left out rather than shown as an error.
 */
export function decodeText(bytes: Uint8Array, cut = false): string {
  const opts = { stream: cut };
  if (bytes[0] === 0xff && bytes[1] === 0xfe) return new TextDecoder("utf-16le").decode(bytes, opts);
  if (bytes[0] === 0xfe && bytes[1] === 0xff) return new TextDecoder("utf-16be").decode(bytes, opts);
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes, opts);
  } catch {
    return new TextDecoder("gb18030").decode(bytes, opts);
  }
}

/** Images we render as <img> thumbnails in chat (HEIC works in Safari, not in Chromium). */
export function isInlineImage(name: string): boolean {
  return /\.(jpe?g|png|gif|webp|bmp|avif|heic|heif)$/i.test(name);
}

export function newClientId(): string {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();
  const b = new Uint8Array(16);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
}

/** A SHA-256 fingerprint the way iPhone certificate details show it ("AB CD …"), 8 pairs to a row. */
export function fingerprintRows(hex: string): string[] {
  const pairs = hex.toUpperCase().match(/.{2}/g) ?? [];
  const rows: string[] = [];
  for (let i = 0; i < pairs.length; i += 8) rows.push(pairs.slice(i, i + 8).join(" "));
  return rows;
}
