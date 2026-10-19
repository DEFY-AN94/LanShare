// One shared EventSource (SSE) connection for the whole page.
// SSE is used instead of WebSocket because it works over the self-signed
// HTTPS origin on every iOS version we care about.

export type ConnState = "connecting" | "open" | "offline" | "unauthorized" | "paused";

type Handler = (data: unknown) => void;

const EVENT_NAMES = ["msg", "files", "devices", "pairing", "pairing_done", "revoked", "paused", "cleared", "pruned", "network", "resync"] as const;

class EventHub {
  private es: EventSource | null = null;
  private handlers = new Map<string, Set<Handler>>();
  private stateHandlers = new Set<(s: ConnState) => void>();
  private retryTimer: number | undefined;
  private everOpened = false;
  private started = false;
  state: ConnState = "connecting";

  start() {
    if (this.started) return;
    this.started = true;
    this.open();
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") this.kick();
    });
    window.addEventListener("online", () => this.kick());
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
    for (const name of EVENT_NAMES) {
      es.addEventListener(name, (e) => {
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
      const reconnect = this.everOpened;
      this.everOpened = true;
      this.setState("open");
      // Anything sent while we were away has to be fetched again.
      if (reconnect) this.dispatch("resync", null);
    };
    es.onerror = () => {
      if (es.readyState === EventSource.CLOSED) {
        // The browser gave up (non-200 response). Find out why.
        this.es = null;
        void this.diagnose();
      } else if (this.state === "open") {
        this.setState("connecting");
      }
    };
  }

  /** Device pages pause on `paused`; the host window keeps its stream. */
  isDeviceStream = true;

  private async diagnose() {
    try {
      const res = await fetch("/api/me", { headers: { "X-LanShare": "1" }, cache: "no-store" });
      if (res.status === 401) {
        this.setState("unauthorized");
        return;
      }
      this.setState(res.status === 503 ? "paused" : "offline");
    } catch {
      this.setState("offline");
    }
    this.scheduleRetry(3000);
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
    this.state = s;
    this.stateHandlers.forEach((fn) => fn(s));
  }

  private dispatch(name: string, data: unknown) {
    this.handlers.get(name)?.forEach((fn) => fn(data));
  }
}

export const events = new EventHub();
