import { useEffect, useRef, useState } from "react";
import { Icon } from "../components/Icon";
import { api } from "../lib/api";
import { guessDeviceName } from "../lib/format";
import type { PairRequestResponse, PairStatus } from "../lib/types";
import { CertGuide } from "./CertGuide";

interface Props {
  /** One-time token from the QR code; null when the user types the 6-digit code. */
  token: string | null;
  notPaired?: boolean;
}

type Stage = "form" | "waiting" | "done" | "failed";

export function PairPage({ token: initialToken, notPaired }: Props) {
  const [token, setToken] = useState(initialToken);
  const [name, setName] = useState(guessDeviceName());
  const [code, setCode] = useState("");
  const [stage, setStage] = useState<Stage>("form");
  const [request, setRequest] = useState<PairRequestResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearInterval(timer.current), []);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await api.post<PairRequestResponse>("/api/pair/request", {
        token: token ?? undefined,
        code: token ? undefined : code.replace(/\D/g, ""),
        name: name.trim() || guessDeviceName(),
      });
      setRequest(res);
      setStage("waiting");
      poll(res.request_id);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const poll = (id: string) => {
    window.clearInterval(timer.current);
    timer.current = window.setInterval(async () => {
      try {
        const { status } = await api.get<{ status: PairStatus }>(`/api/pair/status/${encodeURIComponent(id)}`);
        if (status === "pending") return;
        window.clearInterval(timer.current);
        if (status === "approved") {
          setStage("done");
          window.setTimeout(() => window.location.replace("/"), 900);
        } else {
          setStage("failed");
          setError(status === "rejected" ? "电脑拒绝了这次连接" : "请求已过期，请在电脑上重新生成二维码");
        }
      } catch {
        // transient network error: keep polling
      }
    }, 1500);
  };

  return (
    <div className="pair-page">
      <div className="pair-card card">
        <div className="pair-logo">
          <Icon name="wifi" size={28} />
        </div>
        {stage === "form" && (
          <>
            <h1>{notPaired ? "这台设备还没有配对" : "连接到电脑"}</h1>
            <p className="muted">
              {token
                ? "确认设备名称后点「请求连接」，然后在电脑上点「允许」。"
                : "在电脑端 LanShare 点「添加设备」，扫描二维码；或者在下面输入电脑上显示的 6 位配对码。"}
            </p>
            <form
              className="pair-form"
              onSubmit={(e) => {
                e.preventDefault();
                void submit();
              }}
            >
              {!token && (
                <label className="field">
                  <span className="field-label">配对码</span>
                  <input
                    className="input code-input"
                    inputMode="numeric"
                    autoComplete="one-time-code"
                    maxLength={7}
                    placeholder="000000"
                    value={code}
                    onChange={(e) => setCode(e.target.value.replace(/[^\d]/g, "").slice(0, 6))}
                  />
                </label>
              )}
              <label className="field">
                <span className="field-label">设备名称</span>
                <input className="input" maxLength={40} value={name} onChange={(e) => setName(e.target.value)} />
              </label>
              {error && <p className="text-danger">{error}</p>}
              <button className="btn btn-primary btn-block" disabled={busy || (!token && code.length !== 6)}>
                {busy ? "正在请求…" : "请求连接"}
              </button>
              {token && (
                <button
                  type="button"
                  className="link-btn pair-switch"
                  onClick={() => {
                    setToken(null);
                    setError(null);
                  }}
                >
                  二维码过期了？改用 6 位配对码
                </button>
              )}
            </form>
          </>
        )}
        {stage === "waiting" && request && (
          <>
            <h1>请在电脑上确认</h1>
            <p className="muted">在「{request.host_name}」上核对确认码后点「允许」</p>
            <div className="confirm-code">{request.confirm_code}</div>
            <div className="spinner" aria-label="等待中" />
          </>
        )}
        {stage === "done" && (
          <>
            <h1>配对成功</h1>
            <p className="muted">正在进入…</p>
          </>
        )}
        {stage === "failed" && (
          <>
            <h1>没有连上</h1>
            <p className="text-danger">{error}</p>
            <button
              className="btn btn-primary btn-block"
              onClick={() => {
                // The QR token is single-use; continue with the 6-digit code.
                setToken(null);
                setStage("form");
                setError(null);
              }}
            >
              输入配对码重试
            </button>
            <p className="muted small">也可以在电脑上重新点「添加设备」，再扫一次二维码。</p>
          </>
        )}
      </div>
      {stage !== "done" && (
        <p className="muted small pair-hint">二维码 2 分钟内有效：建议先完成配对，再按下面的步骤安装证书（配对后也能在右上角菜单里找到）。</p>
      )}
      <CertGuide compact />
    </div>
  );
}
