// One shared EventSource (SSE) connection for the whole page.
// SSE is used instead of WebSocket because it works over the self-signed
// HTTPS origin on every iOS version we care about.

export type ConnState = "connecting" | "open" | "offline" | "unauthorized" | "paused";

type Handler = (data: unknown) => void;

const EVENT_NAMES = ["msg", "msg_deleted", "files", "devices", "pairing", "pairing_done", "revoked", "paused", "cleared", "pruned", "network", "shared_root", "resync", "ping"] as const;

/** iOS can leave a stream "connecting" for a minute (no Wi-Fi, cellular on). */
const OPEN_TIMEOUT_MS = 6_000;
const PROBE_TIMEOUT_MS = 3_500;
/** The server pings every 15 s; this long without any event means the stream is dead. */
const STALE_MS = 40_000;
/** After this long hidden, or this long without an event, a resumed page reconnects at once. */
const RESUME_HIDDEN_MS = 10_000;
const RESUME_QUIET_MS = 20_000;

/** Quick retries while someone is looking and Wi-Fi may just be coming back, then slower. */
function retryDelay(offlineFor: number) {
  if (document.visibilityState !== "visible") return 15_000;
  if (offlineFor < 60_000) return 2_000;
  return offlineFor < 300_000 ? 5_000 : 15_000;
}

class EventHub {
  private es: EventSource | null = null;
  private handlers = new Map<string, Set<Handler>>();
  private stateHandlers = new Set<(s: ConnState) => void>();
  private retryTimer: number | undefined;
  private openTimer: number | undefined;
  private lastEventAt = 0;
  private hiddenAt = 0;
  private offlineSince = 0;
  private everOpened = false;
  private started = false;
  state: ConnState = "connecting";

  start() {
    if (this.started) return;
    this.started = true;
    this.open();
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") this.resume();
      else this.hiddenAt = Date.now();
    });
    window.addEventListener("pageshow", () => this.resume());
    window.addEventListener("online", () => this.kick());
    // iOS can leave a dead stream looking open: nothing arriving is the only sign.
    window.setInterval(() => {
      if (document.visibilityState === "visible" && this.es && this.state === "open" && Date.now() - this.lastEventAt > STALE_MS) this.restart();
    }, 5_000);
  }

  /** The page is visible again: a stream that may have died in the background is replaced. */
  private resume() {
    const now = Date.now();
    const away = this.hiddenAt ? now - this.hiddenAt : 0;
    this.hiddenAt = 0;
    if (this.es && (away > RESUME_HIDDEN_MS || now - this.lastEventAt > RESUME_QUIET_MS)) this.restart();
    else this.kick();
  }

  private restart() {
    if (this.state === "unauthorized") return;
    this.close();
    if (this.state === "open") this.setState("connecting");
    window.clearTimeout(this.retryTimer);
    this.open();
  }

  on(name: string, fn: Handler): () => void {
    let set = this.handlers.get(name);
    if (!set) {
      set = new Set();
      this.handlers.set(name, set);
    }
    set.add(fn);
    return () => set.delete(fn);
  }

  onState(fn: (s: ConnState) => void): () => void {
    this.stateHandlers.add(fn);
    return () => this.stateHandlers.delete(fn);
  }

  /** Reconnect right away if the stream is down (e.g. the phone woke up). */
  kick() {
    if (this.es || this.state === "unauthorized") return;
    window.clearTimeout(this.retryTimer);
    this.open();
  }

  markUnauthorized() {
    this.close();
    this.setState("unauthorized");
  }

  private open() {
    const es = new EventSource("/api/events");
    this.es = es;
    window.clearTimeout(this.openTimer);
    this.openTimer = window.setTimeout(() => {
      if (this.es !== es || es.readyState === EventSource.OPEN) return;
      // Stuck: give up on this attempt and find out why.
      es.close();
      this.es = null;
      if (this.state === "open") this.setState("connecting");
      void this.diagnose();
    }, OPEN_TIMEOUT_MS);
    for (const name of EVENT_NAMES) {
      es.addEventListener(name, (e) => {
        this.lastEventAt = Date.now();
        if (name === "ping") return;
        let data: unknown = null;
        try {
          data = JSON.parse((e as MessageEvent<string>).data);
        } catch {
          // ignore malformed payloads
        }
        this.dispatch(name, data);
        if (name === "revoked") this.markUnauthorized();
        if (name === "paused" && (data as { paused?: boolean })?.paused && this.isDeviceStream) {
          this.close();
          this.setState("paused");
          this.scheduleRetry(4000);
        }
      });
    }
    es.onopen = () => {
      window.clearTimeout(this.openTimer);
      this.lastEventAt = Date.now();
      const reconnect = this.everOpened;
      this.everOpened = true;
      this.setState("open");
      // Anything sent while we were away has to be fetched again.
      if (reconnect) this.dispatch("resync", null);
    };
    es.onerror = () => {
      // Never leave reconnecting to the browser: after a dropped connection
      // WebKit can sit in CONNECTING for good. Close it, find out why, and
      // retry on our own schedule.
      window.clearTimeout(this.openTimer);
      if (this.es !== es) return;
      es.close();
      this.es = null;
      if (this.state === "open") this.setState("connecting");
      void this.diagnose();
    };
  }

  /** Device pages pause on `paused`; the host window keeps its stream. */
  isDeviceStream = true;

  private async diagnose() {
    const abort = new AbortController();
    const timer = window.setTimeout(() => abort.abort(), PROBE_TIMEOUT_MS);
    let status = 0;
    try {
      status = (await fetch("/api/me", { headers: { "X-LanShare": "1" }, cache: "no-store", signal: abort.signal })).status;
    } catch {
      // not reachable
    } finally {
      window.clearTimeout(timer);
    }
    // A newer stream opened while this probe ran (Wi-Fi came back): that one decides.
    if (this.es) return;
    if (status === 401) {
      this.setState("unauthorized");
      return;
    }
    // Reachable again: reconnect soon (not at once, in case the stream
    // itself keeps failing while plain requests work).
    if (status >= 200 && status < 300) {
      this.scheduleRetry(1000);
      return;
    }
    this.setState(status === 503 ? "paused" : "offline");
    this.scheduleRetry(retryDelay(this.offlineSince ? Date.now() - this.offlineSince : 0));
  }

  private scheduleRetry(ms: number) {
    window.clearTimeout(this.retryTimer);
    this.retryTimer = window.setTimeout(() => {
      if (!this.es && this.state !== "unauthorized") this.open();
    }, ms);
  }

  private close() {
    this.es?.close();
    this.es = null;
  }

  private setState(s: ConnState) {
    if (this.state === s) return;
    if (s === "offline") this.offlineSince = Date.now();
    else if (s === "open") this.offlineSince = 0;
    this.state = s;
    this.stateHandlers.forEach((fn) => fn(s));
  }

  private dispatch(name: string, data: unknown) {
    this.handlers.get(name)?.forEach((fn) => fn(data));
  }
}

export const events = new EventHub();
