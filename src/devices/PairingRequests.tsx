import { useEffect, useId, useRef, useState } from "react";
import { useDialog } from "../components/Dialog";
import { Icon } from "../components/Icon";
import { useToast } from "../components/Toast";
import { api } from "../lib/api";
import { events } from "../lib/events";
import type { LoginItemInfo, PairRequest } from "../lib/types";
import { useModalFocus } from "../lib/useModalFocus";

/** After the first pairing, offer launch at login once (it stays the user's choice). */
async function suggestLoginItem(confirm: ReturnType<typeof useDialog>["confirm"]) {
  const info = await api.get<LoginItemInfo>("/api/host/login-item");
  if (!info.suggest) return;
  // Only the computer's own window asks this: WebView2 on Windows, WKWebView on a Mac.
  const windows = navigator.userAgent.includes("Windows");
  const yes = await confirm({
    title: windows ? "登录 Windows 时自动启动 LanShare？" : "登录 Mac 时自动启动 LanShare？",
    message: `这样手机随时都能连上电脑。LanShare 会在${windows ? "任务栏通知区域" : "菜单栏"}里运行，不打开窗口；以后可以在设置里更改。`,
    confirmText: "自动启动",
    cancelText: "暂不",
  });
  await (yes ? api.post("/api/host/login-item", { on: true }) : api.post("/api/host/login-item/suggested"));
}

/** Host-wide modal that asks to approve each incoming pairing request. */
export function PairingRequests() {
  const [queue, setQueue] = useState<PairRequest[]>([]);
  const [busy, setBusy] = useState(false);
  const toast = useToast();
  const dialog = useDialog();

  useEffect(() => {
    api
      .get<PairRequest[]>("/api/host/pairing/requests")
      .then(setQueue)
      .catch(() => {});
    const offs = [
      events.on("pairing", (data) => {
        const r = data as PairRequest;
        setQueue((q) => (q.some((x) => x.id === r.id) ? q : [...q, r]));
      }),
      events.on("pairing_done", (data) => {
        const id = (data as { id?: string })?.id;
        setQueue((q) => q.filter((x) => x.id !== id));
      }),
    ];
    return () => offs.forEach((off) => off());
  }, []);

  const current = queue[0];
  if (!current) return null;

  const decide = async (approve: boolean) => {
    setBusy(true);
    try {
      await api.post(`/api/host/pairing/requests/${encodeURIComponent(current.id)}/${approve ? "approve" : "reject"}`);
      if (approve) void suggestLoginItem(dialog.confirm).catch(() => undefined);
    } catch (e) {
      toast((e as Error).message, "error");
    } finally {
      setQueue((q) => q.filter((x) => x.id !== current.id));
      setBusy(false);
    }
  };

  return <RequestDialog key={current.id} request={current} busy={busy} onDecide={(approve) => void decide(approve)} />;
}

/**
 * It opens while the user may be typing elsewhere, so focus goes to the dialog
 * itself and not to a button: a stray Enter neither allows nor refuses. Escape
 * and the backdrop do not close it either; each request is decided.
 */
function RequestDialog({ request, busy, onDecide }: { request: PairRequest; busy: boolean; onDecide: (approve: boolean) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const hintId = useId();
  const codeId = useId();
  useModalFocus(ref);
  return (
    <div className="modal-backdrop">
      <div ref={ref} className="modal dialog pairing-request" role="alertdialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={`${hintId} ${codeId}`} tabIndex={-1}>
        <div className="pair-logo">
          <Icon name="phone" size={26} />
        </div>
        <h2 className="dialog-title" id={titleId}>
          「{request.name}」请求连接
        </h2>
        <p className="muted" id={hintId}>
          请确认手机上显示的确认码和下面一致
        </p>
        <div className="confirm-code" id={codeId}>
          {request.confirm_code}
        </div>
        <p className="muted small">来自 {request.ip}</p>
        <div className="dialog-actions">
          <button className="btn" disabled={busy} onClick={() => onDecide(false)}>
            拒绝
          </button>
          <button className="btn btn-primary" disabled={busy} onClick={() => onDecide(true)}>
            允许
          </button>
        </div>
      </div>
    </div>
  );
}
