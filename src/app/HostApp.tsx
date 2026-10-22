// Desktop window layout (the computer itself, signed in via the local token).

import { useEffect, useMemo, useState } from "react";
import { Chat } from "../chat/Chat";
import { Icon } from "../components/Icon";
import { useToast } from "../components/Toast";
import { DevicesPage, deviceIcon } from "../devices/DevicesPage";
import { AddDeviceModal } from "../devices/AddDeviceModal";
import { PairingRequests } from "../devices/PairingRequests";
import { useDevices } from "../devices/useDevices";
import { FileBrowser } from "../files/FileBrowser";
import { ApiError, api } from "../lib/api";
import { events } from "../lib/events";
import { formatShort } from "../lib/format";
import { navigate, parseHash, replaceRoute, useHashRoute } from "../lib/route";
import type { Me } from "../lib/types";
import { SettingsPage } from "../settings/SettingsPage";

const TABS = [
  { id: "chat", label: "消息", icon: "message" },
  { id: "files", label: "文件", icon: "folder" },
  { id: "devices", label: "设备", icon: "devices" },
  { id: "settings", label: "设置", icon: "settings" },
];

function tabOf(route: string[]): string {
  return TABS.some((t) => t.id === route[0]) ? route[0] : "chat";
}

/** The conversation a route shows, if any. */
function conversationOf(route: string[]): string | null {
  return tabOf(route) === "chat" ? (route[1] ?? null) : null;
}

/** How long a conversation stays in front before it counts as read. */
const SETTLE_MS = 400;

export function HostApp({ me }: { me: Me }) {
  const route = useHashRoute();
  const tab = tabOf(route);
  const [paused, setPaused] = useState(me.paused);
  const { devices } = useDevices();
  // Counted on the computer (messages after the last one read), so they
  // survive reloads and match the Dock badge.
  const unread = useMemo(() => Object.fromEntries((devices ?? []).map((d) => [d.id, d.unread])), [devices]);
  const toast = useToast();
  const openConv = conversationOf(route);

  useEffect(() => {
    if (!route.length) replaceRoute("chat");
  }, [route.length]);

  useEffect(() => {
    const offs = [
      events.on("paused", (d) => setPaused(!!(d as { paused?: boolean })?.paused)),
      // How a menu bar or shortcut action went, while this window is in front.
      events.on("notice", (d) => {
        const n = d as { text?: string; error?: boolean } | null;
        if (n?.text) toast(n.text, n.error ? "error" : "success");
      }),
    ];
    return () => offs.forEach((off) => off());
  }, [toast]);

  // Tells the computer which conversation is in front: it counts as read,
  // and its notifications are taken back. Nothing is in front while the
  // window is hidden or another app is active. Leaving is reported at once;
  // arriving once it has stayed in front a moment, so a conversation that
  // only flashes past (a notification click switching away from it) stays
  // unread. One request at a time, so the computer ends on the latest.
  useEffect(() => {
    let wanted: string | null = null;
    let sent: string | null | undefined;
    let busy = false;
    let pending: { id: string; timer: number } | null = null;
    const front = () => (document.visibilityState === "visible" && document.hasFocus() ? conversationOf(parseHash()) : null);
    const flush = () => {
      if (busy || wanted === sent) return;
      busy = true;
      const device = wanted;
      let again = true;
      api
        .post("/api/host/viewing", { device })
        .then(
          () => {
            sent = device;
          },
          (e) => {
            // A device removed meanwhile is not asked about again; a lost connection is, on the next tick.
            const refused = e instanceof ApiError && e.status >= 400 && e.status < 500;
            sent = refused ? device : undefined;
            again = refused;
          },
        )
        .finally(() => {
          busy = false;
          if (again) flush();
        });
    };
    const update = () => {
      const id = front();
      if (id === null || id !== pending?.id) {
        if (pending) window.clearTimeout(pending.timer);
        pending = null;
      }
      if (id === null) {
        wanted = null;
      } else if (id !== wanted && !pending) {
        wanted = null;
        pending = {
          id,
          timer: window.setTimeout(() => {
            pending = null;
            if (front() === id) wanted = id;
            flush();
          }, SETTLE_MS),
        };
      }
      flush();
    };
    update();
    const timer = window.setInterval(update, 1000);
    const offs = [events.on("resync", () => ((sent = undefined), update()))];
    for (const type of ["focus", "blur", "hashchange"] as const) window.addEventListener(type, update);
    document.addEventListener("visibilitychange", update);
    return () => {
      window.clearInterval(timer);
      if (pending) window.clearTimeout(pending.timer);
      offs.forEach((off) => off());
      for (const type of ["focus", "blur", "hashchange"] as const) window.removeEventListener(type, update);
      document.removeEventListener("visibilitychange", update);
    };
  }, []);

  const totalUnread = Object.values(unread).reduce((a, b) => a + b, 0);
  useEffect(() => {
    document.title = totalUnread ? `(${totalUnread}) LanShare` : "LanShare";
  }, [totalUnread]);

  // Default to the most recent conversation when none is selected.
  const sorted = useMemo(
    () => [...(devices ?? [])].sort((a, b) => (b.last_message?.id ?? 0) - (a.last_message?.id ?? 0) || b.created_at - a.created_at),
    [devices],
  );
  useEffect(() => {
    if (tab === "chat" && !route[1] && sorted.length) replaceRoute("chat", sorted[0].id);
  }, [tab, route, sorted]);

  const resume = async () => {
    try {
      await api.post("/api/host/paused", { paused: false });
      setPaused(false);
    } catch (e) {
      toast((e as Error).message, "error");
    }
  };

  return (
    <div className="host">
      <nav className="rail" aria-label="主导航">
        {TABS.map((t) => (
          <button key={t.id} className={`rail-item ${tab === t.id ? "active" : ""}`} onClick={() => navigate(t.id)} title={t.label}>
            <Icon name={t.icon} size={22} />
            <span>{t.label}</span>
            {t.id === "chat" && totalUnread > 0 && <span className="badge">{totalUnread > 99 ? "99+" : totalUnread}</span>}
          </button>
        ))}
      </nav>
      <main className="host-main">
        {paused && (
          <div className="banner banner-warn">
            <Icon name="pause" size={16} />
            <span>共享已暂停，手机暂时无法访问这台电脑</span>
            <button className="btn btn-small" onClick={() => void resume()}>
              恢复共享
            </button>
          </div>
        )}
        {tab === "chat" && <HostChat devices={sorted} openId={openConv} unread={unread} />}
        {tab === "files" && <FileBrowser path={route.slice(1).join("/")} isHost onNavigate={(p) => navigate("files", ...p.split("/"))} />}
        {tab === "devices" && <DevicesPage />}
        {tab === "settings" && <SettingsPage />}
      </main>
      <PairingRequests />
    </div>
  );
}

function HostChat({
  devices,
  openId,
  unread,
}: {
  devices: ReturnType<typeof useDevices>["devices"] & object;
  openId: string | null;
  unread: Record<string, number>;
}) {
  const [adding, setAdding] = useState(false);
  const open = devices.find((d) => d.id === openId) ?? null;

  if (!devices.length) {
    return (
      <div className="empty-state">
        <Icon name="phone" size={44} />
        <p>还没有配对的设备</p>
        <p className="muted">用 iPhone 扫码配对后，就可以在这里互发消息和文件</p>
        <button className="btn btn-primary" onClick={() => setAdding(true)}>
          <Icon name="qr" size={16} />
          <span>添加设备</span>
        </button>
        {adding && <AddDeviceModal onClose={() => setAdding(false)} />}
      </div>
    );
  }

  return (
    <div className="host-chat">
      <aside className="conv-list">
        {devices.map((d) => {
          const last = d.last_message;
          const preview = last ? (last.kind === "text" ? last.text : `[文件] ${last.file?.name ?? ""}`) : "还没有消息";
          return (
            <button key={d.id} className={`conv-item ${d.id === openId ? "active" : ""}`} onClick={() => navigate("chat", d.id)}>
              <span className="conv-avatar">
                <Icon name={deviceIcon(d.name)} size={20} />
                <span className={`dot ${d.online ? "online" : ""}`} />
              </span>
              <span className="conv-body">
                <span className="conv-top">
                  <span className="conv-name ellipsis">{d.name}</span>
                  {last && <span className="conv-time">{formatShort(last.created_at)}</span>}
                </span>
                <span className="conv-preview ellipsis">{preview}</span>
              </span>
              {!!unread[d.id] && <span className="badge">{unread[d.id]}</span>}
            </button>
          );
        })}
      </aside>
      {open ? (
        <Chat
          key={open.id}
          deviceId={open.id}
          isHost
          header={
            <div className="chat-header">
              <span className="chat-title">{open.name}</span>
              <span className={`status ${open.online ? "online" : ""}`}>{open.online ? "在线" : "离线（消息会在它打开网页后送达）"}</span>
            </div>
          }
        />
      ) : (
        <div className="empty-state muted">选择左侧的设备开始聊天</div>
      )}
    </div>
  );
}
