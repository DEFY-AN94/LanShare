// Opening, downloading and sharing files, split by who is looking:
// the host window asks the server to use the OS; phones use the browser.

import { api } from "./api";

/** Opens with the default app. Apps/scripts are only revealed in the folder. */
export async function hostOpen(path: string, notify?: (msg: string) => void) {
  const res = await api.post<{ action?: string }>("/api/host/open", { path });
  if (res?.action === "revealed") notify?.("为了安全，程序和脚本类文件不会直接打开，已在文件夹中显示");
  return res;
}

export function hostReveal(path: string) {
  return api.post("/api/host/reveal", { path });
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

export function canShareFiles(): boolean {
  return typeof navigator.share === "function" && typeof navigator.canShare === "function";
}

/** Downloads into memory so the share sheet can offer "存储图像/存储视频". */
export async function fetchAsFile(url: string, name: string, mime: string): Promise<File> {
  const res = await fetch(url, { credentials: "same-origin" });
  if (!res.ok) throw new Error(`下载失败（${res.status}）`);
  const blob = await res.blob();
  return new File([blob], name, { type: mime || blob.type || "application/octet-stream" });
}
