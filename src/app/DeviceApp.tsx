// Phone layout (paired device in Safari).

import { useEffect, useRef, useState } from "react";
import { Chat } from "../chat/Chat";
import { startOutbox } from "../chat/store";
import { ActionSheet } from "../components/ActionSheet";
import { useDialog } from "../components/Dialog";
import { Icon } from "../components/Icon";
import { useToast } from "../components/Toast";
import { FileBrowser } from "../files/FileBrowser";
import { api } from "../lib/api";
import { events, type ConnState } from "../lib/events";
import { navigate, replaceRoute, useHashRoute } from "../lib/route";
import { clearConversationData } from "../lib/storage";
import type { Me, Message } from "../lib/types";
import { CertGuide } from "../pair/CertGuide";

export function useConnState(): ConnState {
  const [state, setState] = useState(events.state);
  useEffect(() => {
    const off = events.onState(setState);
    // The stream may have opened between the first render and this effect.
    setState(events.state);
    return off;
  }, []);
  return state;
}

export function DeviceApp({ me }: { me: Me }) {
  const route = useHashRoute();
  const tab = route[0] === "files" || route[0] === "cert" ? route[0] : "chat";
  const conn = useConnState();
  const [menu, setMenu] = useState(false);
  const [unread, setUnread] = useState(0);
  const dialog = useDialog();
  const toast = useToast();
  const tabRef = useRef(tab);
  tabRef.current = tab;

  useEffect(() => {
    if (!route.length) replaceRoute("chat");
  }, [route.length]);

  // Texts queued before iOS reloaded the page go out even if the chat tab is never opened.
  useEffect(() => startOutbox(), []);

  // In the Home Screen app, ask iOS not to clear drafts and unsent messages when space runs low.
  useEffect(() => {
    const standalone = window.matchMedia("(display-mode: standalone)").matches || (navigator as Navigator & { standalone?: boolean }).standalone;
    if (standalone) void navigator.storage?.persist?.().catch(() => undefined);
  }, []);

  useEffect(
    () =>
      events.on("msg", (d) => {
        if ((d as Message).from_host && tabRef.current !== "chat") setUnread((n) => n + 1);
      }),
    [],
  );
  useEffect(() => {
    if (tab === "chat") setUnread(0);
  }, [tab]);

  const unpair = async () => {
    const ok = await dialog.confirm({
      title: "解除配对？",
      message: "这台设备会立即断开，和电脑的聊天记录也会被删除。之后需要重新扫码才能连接。",
      confirmText: "解除配对",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.post("/api/me/unpair");
      clearConversationData();
      window.location.replace("/");
    } catch (e) {
      toast((e as Error).message, "error");
    }
  };

  const offline = conn === "offline" || conn === "paused" || conn === "unauthorized";
  // Messages can still be written and queued while the computer is unreachable.
  const chatOffline = conn === "offline" || conn === "paused";

  return (
    <div className="device app-viewport">
      <header className="topbar">
        {tab === "cert" ? (
          <button className="icon-btn" aria-label="返回" onClick={() => history.back()}>
            <Icon name="chevron-left" />
          </button>
        ) : (
          <span className="topbar-spacer" />
        )}
        <div className="topbar-title">
          <span className="ellipsis">{me.host_name}</span>
          <span className={`status ${conn === "open" ? "online" : ""}`}>
            {conn === "open"
              ? "已连接"
              : conn === "paused"
                ? "电脑已暂停共享"
                : conn === "offline"
                  ? "连接中断，正在重连…"
                  : conn === "unauthorized"
                    ? "已解除配对"
                    : "正在连接…"}
          </span>
        </div>
        <button className="icon-btn" aria-label="更多" onClick={() => setMenu(true)}>
          <Icon name="more" />
        </button>
      </header>

      <main className="device-main">
        {tab === "chat" && (
          <Chat
            deviceId={null}
            isHost={false}
            disabled={conn === "unauthorized"}
            offline={chatOffline}
            canTrashFiles={me.can_write}
            emptyHint="在这里给电脑发消息或文件"
          />
        )}
        {tab === "files" && <FileBrowser path={route.slice(1).join("/")} isHost={false} disabled={offline} onNavigate={(p) => navigate("files", ...p.split("/"))} />}
        {tab === "cert" && (
          <div className="page">
            <CertGuide />
          </div>
        )}
      </main>

      <nav className="tabbar">
        <button className={`tab ${tab === "chat" ? "active" : ""}`} onClick={() => navigate("chat")}>
          <span className="tab-icon">
            <Icon name="message" size={24} />
            {unread > 0 && <span className="badge">{unread > 99 ? "99+" : unread}</span>}
          </span>
          <span>消息</span>
        </button>
        <button className={`tab ${tab === "files" ? "active" : ""}`} onClick={() => navigate("files")}>
          <Icon name="folder" size={24} />
          <span>文件</span>
        </button>
      </nav>

      {menu && (
        <ActionSheet
          title={me.device_name ?? "这台设备"}
          subtitle={`已连接到「${me.host_name}」${me.can_write ? "" : " · 文件只读"}`}
          onClose={() => setMenu(false)}
          actions={[
            { label: "安装电脑的证书", icon: "shield", onSelect: () => navigate("cert") },
            { label: "解除配对", icon: "x", danger: true, onSelect: () => void unpair() },
          ]}
        />
      )}
    </div>
  );
}
