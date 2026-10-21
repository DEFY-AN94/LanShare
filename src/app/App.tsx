import { useEffect, useRef, useState } from "react";
import { Icon } from "../components/Icon";
import { api, ApiError, onUnauthorized, PROBE_TIMEOUT_MS } from "../lib/api";
import { events } from "../lib/events";
import { clearConversationData } from "../lib/storage";
import type { Me } from "../lib/types";
import { PairPage } from "../pair/PairPage";
import { DeviceApp } from "./DeviceApp";
import { HostApp } from "./HostApp";

type State =
  | { status: "loading" }
  | { status: "ready"; me: Me }
  | { status: "pair"; token: string | null }
  | { status: "unpaired" }
  | { status: "paused" }
  | { status: "error"; message: string; offline: boolean };

/** While the computer cannot be reached, try again this often (only while visible). */
const RETRY_MS = 2_000;

/** The phone keeps a copy of this page (public/sw.js); it may be what is showing now. */
const hasOfflineCopy = () => !!navigator.serviceWorker?.controller;

/**
 * Opens the page straight from the computer, past the stored copy (sw.js
 * leaves ?direct to the network). After the computer's certificate changed,
 * only then does Safari show its own warning and the way to the new
 * certificate. The copy itself stays, so while the computer is away the next
 * start still opens to this screen rather than Safari's error page.
 */
function openFromComputer() {
  window.location.replace(`/?direct=${Date.now()}`);
}

function initialState(): State {
  if (new URLSearchParams(window.location.search).has("direct")) {
    history.replaceState(null, "", window.location.pathname + window.location.hash);
  }
  if (window.location.pathname === "/pair") {
    const token = new URLSearchParams(window.location.search).get("t");
    // Drop the one-time token from the address bar and history right away.
    history.replaceState(null, "", "/pair");
    return { status: "pair", token };
  }
  return { status: "loading" };
}

export function App() {
  const [state, setState] = useState<State>(initialState);
  const booting = useRef(false);

  const boot = async () => {
    if (booting.current) return;
    booting.current = true;
    try {
      // A short limit: without Wi-Fi the request may otherwise hang for a minute.
      const me = await api.get<Me>("/api/me", PROBE_TIMEOUT_MS);
      events.isDeviceStream = me.kind === "device";
      events.start();
      setState({ status: "ready", me });
    } catch (e) {
      const err = e as ApiError;
      if (err.status === 401) setState({ status: "unpaired" });
      else if (err.status === 503) setState({ status: "paused" });
      else setState({ status: "error", message: err.message, offline: err.status === 0 });
    } finally {
      booting.current = false;
    }
  };

  useEffect(() => {
    if (state.status === "loading") void boot();
    const toUnpaired = () => {
      // Nothing this pairing typed or queued stays on the phone.
      clearConversationData();
      setState((s) => (s.status === "pair" ? s : { status: "unpaired" }));
    };
    const offs = [
      onUnauthorized(() => {
        events.markUnauthorized();
        toUnpaired();
      }),
      // Removed by the computer (SSE `revoked`) or a 401 seen by the event stream.
      events.onState((s) => s === "unauthorized" && toUnpaired()),
    ];
    return () => offs.forEach((off) => off());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // While paused or unreachable, keep checking so the app comes back on its
  // own: when Wi-Fi returns, when the computer wakes up, when sharing resumes.
  const waiting = state.status === "paused" || (state.status === "error" && state.offline);
  useEffect(() => {
    if (!waiting) return;
    const t = window.setInterval(() => document.visibilityState === "visible" && void boot(), RETRY_MS);
    const now = () => void boot();
    const onVisible = () => document.visibilityState === "visible" && now();
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("online", now);
    return () => {
      window.clearInterval(t);
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("online", now);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [waiting]);

  switch (state.status) {
    case "loading":
      return (
        <div className="splash">
          <span className="muted small">正在连接电脑…</span>
        </div>
      );
    case "pair":
      return <PairPage token={state.token} />;
    case "unpaired":
      return <PairPage token={null} notPaired />;
    case "ready":
      return state.me.kind === "host" ? <HostApp me={state.me} /> : <DeviceApp me={state.me} />;
    case "paused":
    case "error":
      return (
        <div className="pair-page">
          <div className="pair-card card">
            <div className="pair-logo">
              <Icon name={state.status === "paused" ? "pause" : "alert"} size={28} />
            </div>
            <h1>{state.status === "paused" ? "电脑端已暂停共享" : "连接不上电脑"}</h1>
            <p className="muted">
              {state.status === "paused" ? "请在电脑上的 LanShare 里点「恢复共享」。" : state.message}
            </p>
            {state.status === "error" && state.offline && (
              <ul className="cause-list muted small">
                <li>手机和电脑要连同一个 Wi-Fi</li>
                <li>电脑上的 LanShare 要在运行（菜单栏或任务栏里有它的图标）</li>
                <li>电脑可能已睡眠：唤醒电脑后这里会自动连上</li>
                <li>手机上的 VPN 可能会拦住局域网连接</li>
                {hasOfflineCopy() && <li>电脑上重置过证书或重装过 LanShare：点下面的「直接从电脑打开」，再按 Safari 的提示操作</li>}
              </ul>
            )}
            {waiting && <p className="muted small">正在自动重试…</p>}
            <button className="btn btn-primary btn-block" onClick={() => void boot()}>
              重试
            </button>
            {state.status === "error" && state.offline && hasOfflineCopy() && (
              <button className="btn btn-block" onClick={openFromComputer}>
                直接从电脑打开
              </button>
            )}
          </div>
        </div>
      );
  }
}
