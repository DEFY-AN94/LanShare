import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { useToast } from "../components/Toast";
import { api } from "../lib/api";
import { events } from "../lib/events";
import type { PairRequest } from "../lib/types";

/** Host-wide modal that asks to approve each incoming pairing request. */
export function PairingRequests() {
  const [queue, setQueue] = useState<PairRequest[]>([]);
  const [busy, setBusy] = useState(false);
  const toast = useToast();

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
    } catch (e) {
      toast((e as Error).message, "error");
    } finally {
      setQueue((q) => q.filter((x) => x.id !== current.id));
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop">
      <div className="modal dialog pairing-request" role="alertdialog" aria-modal="true">
        <div className="pair-logo">
          <Icon name="phone" size={26} />
        </div>
        <h2 className="dialog-title">「{current.name}」请求连接</h2>
        <p className="muted">请确认手机上显示的确认码和下面一致</p>
        <div className="confirm-code">{current.confirm_code}</div>
        <p className="muted small">来自 {current.ip}</p>
        <div className="dialog-actions">
          <button className="btn" disabled={busy} onClick={() => void decide(false)}>
            拒绝
          </button>
          <button className="btn btn-primary" disabled={busy} onClick={() => void decide(true)}>
            允许
          </button>
        </div>
      </div>
    </div>
  );
}
