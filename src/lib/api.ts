// Thin fetch/XHR wrappers. Every state-changing request carries the
// `X-LanShare` header, which the server requires as a CSRF guard.

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

const unauthorizedListeners = new Set<() => void>();

/** Called when any request comes back 401 (pairing revoked or cookie lost). */
export function onUnauthorized(fn: () => void): () => void {
  unauthorizedListeners.add(fn);
  return () => unauthorizedListeners.delete(fn);
}

async function errorMessage(res: Response): Promise<string> {
  try {
    const body = await res.json();
    if (body && typeof body.error === "string") return body.error;
  } catch {
    // not JSON
  }
  if (res.status === 503) return "电脑端已暂停共享";
  return `请求失败（${res.status}）`;
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { "X-LanShare": "1" };
  let payload: string | undefined;
  if (body !== undefined) {
    headers["Content-Type"] = "application/json";
    payload = JSON.stringify(body);
  }
  let res: Response;
  try {
    res = await fetch(path, { method, headers, body: payload, credentials: "same-origin", cache: "no-store" });
  } catch {
    throw new ApiError(0, "连接不上电脑，请检查是否在同一个 Wi-Fi");
  }
  if (!res.ok) {
    if (res.status === 401) unauthorizedListeners.forEach((fn) => fn());
    throw new ApiError(res.status, await errorMessage(res));
  }
  if (res.status === 204) return undefined as T;
  const type = res.headers.get("content-type") ?? "";
  return (type.includes("application/json") ? await res.json() : undefined) as T;
}

export const api = {
  get: <T>(path: string) => request<T>("GET", path),
  post: <T>(path: string, body?: unknown) => request<T>("POST", path, body ?? {}),
  patch: <T>(path: string, body: unknown) => request<T>("PATCH", path, body),
  del: <T>(path: string) => request<T>("DELETE", path),
};

/** Builds a query string, skipping null/undefined values. */
export function qs(params: Record<string, string | number | boolean | null | undefined>): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(params)) {
    if (v === null || v === undefined) continue;
    parts.push(`${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`);
  }
  return parts.length ? `?${parts.join("&")}` : "";
}

export interface UploadHandle<T> {
  promise: Promise<T>;
  abort: () => void;
}

/**
 * Streams a file as the raw request body (no multipart) so the server can
 * write it straight to disk. Uses XHR because fetch has no upload progress.
 */
export function uploadFile<T>(
  url: string,
  file: Blob,
  onProgress: (loaded: number, total: number) => void,
): UploadHandle<T> {
  const xhr = new XMLHttpRequest();
  const promise = new Promise<T>((resolve, reject) => {
    xhr.open("POST", url);
    xhr.setRequestHeader("X-LanShare", "1");
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    xhr.upload.onprogress = (e) => onProgress(e.loaded, e.lengthComputable ? e.total : file.size);
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          resolve(JSON.parse(xhr.responseText) as T);
        } catch {
          reject(new ApiError(xhr.status, "服务器返回了无法识别的内容"));
        }
        return;
      }
      if (xhr.status === 401) unauthorizedListeners.forEach((fn) => fn());
      let msg = `上传失败（${xhr.status}）`;
      try {
        const body = JSON.parse(xhr.responseText);
        if (body?.error) msg = body.error;
      } catch {
        // keep default
      }
      reject(new ApiError(xhr.status, msg));
    };
    xhr.onerror = () => reject(new ApiError(0, "上传中断，请检查网络"));
    xhr.onabort = () => reject(new ApiError(-1, "已取消"));
    xhr.send(file);
  });
  return { promise, abort: () => xhr.abort() };
}
