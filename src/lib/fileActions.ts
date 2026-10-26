// Opening, downloading and sharing files, split by who is looking:
// the host window asks the server to use the OS; phones use the browser.

import { api, errorMessage } from "./api";

const REVEALED = "为了安全，程序、脚本、安装包、磁盘映像、描述文件和证书不会直接打开，已在文件夹中显示";

/** Opens with the default app. Anything that would run, mount or install is only revealed in the folder. */
export async function hostOpen(path: string, notify?: (msg: string) => void) {
  const res = await api.post<{ action?: string }>("/api/host/open", { path });
  if (res?.action === "revealed") notify?.(REVEALED);
  return res;
}

/** A chat attachment, wherever it was saved (also in an earlier shared folder). */
export async function hostOpenMessageFile(id: number, notify?: (msg: string) => void) {
  const res = await api.post<{ action?: string }>(`/api/host/messages/${id}/open`);
  if (res?.action === "revealed") notify?.(REVEALED);
  return res;
}

export function hostReveal(path: string) {
  return api.post("/api/host/reveal", { path });
}

/** Shows a chat attachment in its folder, wherever it was saved. */
export function hostRevealMessageFile(id: number) {
  return api.post(`/api/host/messages/${id}/reveal`);
}

export function hostOpenUrl(url: string) {
  return api.post("/api/host/open-url", { url });
}

/** Triggers a browser download (saved to the Files app on iOS). */
export function browserDownload(url: string, name: string) {
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.rel = "noopener";
  document.body.appendChild(a);
  a.click();
  a.remove();
}

export function browserPreview(url: string) {
  window.open(url, "_blank", "noopener");
}

/**
 * The Home Screen app. It has no browser controls, so a new window or a
 * download page opened from it has no way back to LanShare.
 */
export function isStandalone(): boolean {
  return window.matchMedia?.("(display-mode: standalone)").matches || (navigator as Navigator & { standalone?: boolean }).standalone === true;
}

export function canShareFiles(): boolean {
  return typeof navigator.share === "function" && typeof navigator.canShare === "function";
}

/** The Home Screen app saves larger files through Safari instead: the share sheet needs the whole file in memory. */
export const SHARE_LIMIT = 200 * 1024 * 1024;

/** The same file served for showing in the page rather than as a download. */
export function inlineUrl(url: string): string {
  return `${url}${url.includes("?") ? "&" : "?"}inline=1`;
}

/** Downloads into memory so the share sheet can offer "保存图像", "保存视频" or "保存到“文件”". */
export async function fetchAsFile(url: string, name: string, mime: string, signal?: AbortSignal): Promise<File> {
  const res = await fetch(url, { credentials: "same-origin", signal });
  if (!res.ok) throw new Error(`下载失败：${await errorMessage(res)}`);
  const blob = await res.blob();
  return new File([blob], name, { type: mime || blob.type || "application/octet-stream" });
}

/** How much of a text file the viewer shows. */
export const TEXT_LIMIT = 1024 * 1024;

/** The start of a text file, up to TEXT_LIMIT bytes. */
export async function fetchTextStart(url: string, signal?: AbortSignal): Promise<Uint8Array> {
  const res = await fetch(url, { credentials: "same-origin", signal, headers: { Range: `bytes=0-${TEXT_LIMIT - 1}` } });
  // An empty file has no byte 0 to start from.
  if (res.status === 416) return new Uint8Array();
  if (!res.ok) throw new Error(`读取失败：${await errorMessage(res)}`);
  return new Uint8Array(await res.arrayBuffer()).subarray(0, TEXT_LIMIT);
}
