import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { useToast } from "../components/Toast";
import { CopyButton } from "../chat/MessageBubble";
import { api } from "../lib/api";
import { events } from "../lib/events";
import type { PairingSession } from "../lib/types";

export function AddDeviceModal({ onClose }: { onClose: () => void }) {
  const [session, setSession] = useState<PairingSession | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState(0);
  const [now, setNow] = useState(Date.now());
  const toast = useToast();

  const create = async () => {
    setError(null);
    try {
      setSession(await api.post<PairingSession>("/api/host/pairing"));
      setSelected(0);
    } catch (e) {
      setError((e as Error).message);
    }
  };

  useEffect(() => {
    void create();
    const tick = window.setInterval(() => setNow(Date.now()), 1000);
    const off = events.on("pairing_done", (data) => {
      const d = data as { status?: string; name?: string };
      if (d?.status === "approved") {
        toast(`已连接「${d.name ?? "新设备"}」`, "success");
        onClose();
      }
    });
    return () => {
      window.clearInterval(tick);
      off();
      // Closing the panel invalidates the QR code right away.
      void api.del("/api/host/pairing").catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const remaining = session ? Math.max(0, Math.ceil((session.expires_at - now) / 1000)) : 0;
  const expired = !!session && remaining === 0;
  const current = session?.urls[selected];
  const origin = current ? new URL(current.url).origin : "";

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal add-device" role="dialog" aria-modal="true">
        <div className="modal-head">
          <h2>添加设备</h2>
          <button className="icon-btn" aria-label="关闭" onClick={onClose}>
            <Icon name="x" />
          </button>
        </div>
        {error && <p className="text-danger">{error}</p>}
        {session && session.urls.length === 0 && (
          <p className="text-danger">没有找到可用的局域网地址。请确认电脑已连接 Wi-Fi 或网线。</p>
        )}
        {current && (
          <div className="add-device-body">
            <div className={`qr ${expired ? "is-expired" : ""}`}>
              <img src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(current.qr_svg)}`} alt="配对二维码" />
              {expired && (
                <button className="btn btn-primary qr-refresh" onClick={() => void create()}>
                  <Icon name="refresh" size={16} /> 重新生成
                </button>
              )}
            </div>
            <div className="add-device-info">
              <ol className="steps">
                <li>iPhone 和这台电脑连同一个 Wi-Fi</li>
                <li>用 iPhone 相机扫描二维码，在 Safari 中打开</li>
                <li>
                  首次会提示「不安全」，点 显示详细信息 › 访问此网站（或按页面引导安装证书）
                </li>
                <li>在手机上点「请求连接」，然后在这里核对确认码并允许</li>
              </ol>
              <div className="pair-code-box">
                <span className="muted small">扫不了码？在手机 Safari 打开下面的地址，输入配对码</span>
                <div className="pair-origin">
                  <code>{origin}</code>
                  <CopyButton text={origin} />
                </div>
                <div className="pair-code">{session!.code.replace(/(\d{3})(\d{3})/, "$1 $2")}</div>
              </div>
              {session!.urls.length > 1 && (
                <label className="field">
                  <span className="field-label">使用的网络地址</span>
                  <select className="input" value={selected} onChange={(e) => setSelected(Number(e.target.value))}>
                    {session!.urls.map((u, i) => (
                      <option key={u.url} value={i}>
                        {u.ip}（{u.iface}）
                      </option>
                    ))}
                  </select>
                </label>
              )}
              <p className="muted small">{expired ? "二维码已过期" : `二维码 ${remaining} 秒后过期，只能使用一次`}</p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
