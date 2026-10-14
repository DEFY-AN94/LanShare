import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { api, ApiError, onUnauthorized } from "../lib/api";
import { events } from "../lib/events";
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
  | { status: "error"; message: string };

function initialState(): State {
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

  const boot = async () => {
    try {
      const me = await api.get<Me>("/api/me");
      events.isDeviceStream = me.kind === "device";
      events.start();
      setState({ status: "ready", me });
    } catch (e) {
      const err = e as ApiError;
      if (err.status === 401) setState({ status: "unpaired" });
      else if (err.status === 503) setState({ status: "paused" });
      else setState({ status: "error", message: err.message });
    }
  };

  useEffect(() => {
    if (state.status === "loading") void boot();
    return onUnauthorized(() => {
      events.markUnauthorized();
      setState((s) => (s.status === "pair" ? s : { status: "unpaired" }));
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  switch (state.status) {
    case "loading":
      return <div className="splash" />;
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
            <button className="btn btn-primary btn-block" onClick={() => void boot()}>
              重试
            </button>
          </div>
        </div>
      );
  }
}
