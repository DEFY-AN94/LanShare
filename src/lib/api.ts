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

/** The server's own message for a failed response, if it sent one. */
export async function errorMessage(res: Response): Promise<string> {
  try {
    const body = await res.json();
    if (body && typeof body.error === "string") return body.error;
  } catch {
    // not JSON
  }
  if (res.status === 503) return "电脑端已暂停共享";
  return `请求失败（${res.status}）`;
}

export const OFFLINE_MESSAGE = "连接不上电脑，请检查是否在同一个 Wi-Fi";

/** Without Wi-Fi a request to the computer can hang for a minute on cellular data. */
const DEFAULT_TIMEOUT_MS = 30_000;
/** Checks whether the computer is reachable at all. */
export const PROBE_TIMEOUT_MS = 3_500;

async function request<T>(method: string, path: string, body?: unknown, timeoutMs = DEFAULT_TIMEOUT_MS): Promise<T> {
  const headers: Record<string, string> = { "X-LanShare": "1" };
  let payload: string | undefined;
  if (body !== undefined) {
    headers["Content-Type"] = "application/json";
    payload = JSON.stringify(body);
  }
  const abort = new AbortController();
  const timer = window.setTimeout(() => abort.abort(), timeoutMs);
  let res: Response;
  try {
    res = await fetch(path, { method, headers, body: payload, credentials: "same-origin", cache: "no-store", signal: abort.signal });
  } catch {
    throw new ApiError(0, OFFLINE_MESSAGE);
  } finally {
    window.clearTimeout(timer);
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
  get: <T>(path: string, timeoutMs?: number) => request<T>("GET", path, undefined, timeoutMs),
  post: <T>(path: string, body?: unknown, timeoutMs?: number) => request<T>("POST", path, body ?? {}, timeoutMs),
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

/** An upload with no progress this long while visible has stalled (dead connection).
 * Safari reports upload progress in large, uneven steps on slow Wi-Fi, so this is generous. */
const STALL_MS = 60_000;
/** Once the whole body is sent, the computer still writes and checks it. */
const RESPONSE_MS = 60_000;
/** After the page comes back, a stalled upload gets this long to move again. */
const RESUME_GRACE_MS = 10_000;

/**
 * Streams a file as the raw request body (no multipart) so the server can
 * write it straight to disk. Uses XHR because fetch has no upload progress.
 * A stalled upload is stopped and fails like a lost connection (status 0),
 * so the caller can try again.
 */
export function uploadFile<T>(
  url: string,
  file: Blob,
  onProgress: (loaded: number, total: number) => void,
): UploadHandle<T> {
  const xhr = new XMLHttpRequest();
  let lastProgress = Date.now();
  let bodySent = false;
  let stalled = false;
  const watch = window.setInterval(() => {
    if (document.visibilityState !== "visible") return;
    if (Date.now() - lastProgress > (bodySent ? RESPONSE_MS : STALL_MS)) {
      stalled = true;
      xhr.abort();
    }
  }, 2_000);
  const onVisible = () => {
    if (document.visibilityState === "visible") lastProgress = Math.max(lastProgress, Date.now() - (STALL_MS - RESUME_GRACE_MS));
  };
  document.addEventListener("visibilitychange", onVisible);
  const stop = () => {
    window.clearInterval(watch);
    document.removeEventListener("visibilitychange", onVisible);
  };
  const promise = new Promise<T>((resolve, reject) => {
    xhr.open("POST", url);
    xhr.setRequestHeader("X-LanShare", "1");
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    xhr.upload.onprogress = (e) => {
      lastProgress = Date.now();
      onProgress(e.loaded, e.lengthComputable ? e.total : file.size);
    };
    xhr.upload.onload = () => {
      bodySent = true;
      lastProgress = Date.now();
    };
    xhr.onloadend = stop;
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
    xhr.onabort = () => reject(stalled ? new ApiError(0, "上传停住了，连上后会自动重试") : new ApiError(-1, "已取消"));
    // A slice, not the File itself: on iOS 26.5 and later, a page controlled
    // by a service worker can send a picked File as an empty body (WebKit bug
    // 319985). The server also checks the size given in the URL.
    xhr.send(file.slice(0, file.size, file.type));
  });
  return { promise, abort: () => xhr.abort() };
}
