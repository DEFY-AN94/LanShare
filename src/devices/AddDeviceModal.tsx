import { type ReactNode, useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { useToast } from "../components/Toast";
import { CopyButton } from "../chat/MessageBubble";
import { api } from "../lib/api";
import { events } from "../lib/events";
import { fingerprintRows } from "../lib/format";
import type { CertSetup, PairingSession, PairingUrl } from "../lib/types";

/**
 * 添加设备 in two steps: install this computer's root certificate on the
 * iPhone (recommended; its QR code carries no secret), then pair. Step 1 can
 * be skipped, and the 2-minute pairing code starts only with step 2.
 */
export function AddDeviceModal({ onClose }: { onClose: () => void }) {
  const [step, setStep] = useState<"cert" | "pair">("cert");
  const [skipped, setSkipped] = useState(false);
  const [setup, setSetup] = useState<CertSetup | null>(null);
  const [session, setSession] = useState<PairingSession | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The address both steps use, by IP: each step gets its own list.
  const [ip, setIp] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());
  const toast = useToast();

  const create = async () => {
    setError(null);
    try {
      setSession(await api.post<PairingSession>("/api/host/pairing"));
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const toPairing = (skip: boolean) => {
    setSkipped(skip);
    setStep("pair");
    void create();
  };

  const backToCert = () => {
    setStep("cert");
    setSession(null);
    // The pairing code shown so far stops working.
    void api.del("/api/host/pairing").catch(() => {});
  };

  useEffect(() => {
    // Again when sharing is paused or resumed, or the addresses change.
    const loadSetup = () => api.get<CertSetup>("/api/host/cert-setup").then(setSetup, (e: Error) => setError(e.message));
    void loadSetup();
    const tick = window.setInterval(() => setNow(Date.now()), 1000);
    const offs = [
      events.on("pairing_done", (data) => {
        const d = data as { status?: string; name?: string };
        if (d?.status === "approved") {
          toast(`已连接「${d.name ?? "新设备"}」`, "success");
          onClose();
        }
      }),
      ...["paused", "network", "resync"].map((name) => events.on(name, () => void loadSetup())),
    ];
    return () => {
      window.clearInterval(tick);
      offs.forEach((off) => off());
      // Closing the panel invalidates the QR code right away.
      void api.del("/api/host/pairing").catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const urls = (step === "cert" ? setup?.urls : session?.urls) ?? [];
  const current = urls.find((u) => u.ip === ip) ?? urls[0];
  const warning = setup?.network_warning ?? session?.network_warning;

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
        {warning && (
          <div className="banner banner-warn banner-block">
            <Icon name="alert" size={16} />
            <span>{warning}</span>
          </div>
        )}
        {setup?.paused && (
          <div className="banner banner-warn banner-block">
            <Icon name="pause" size={16} />
            <span>共享已暂停：先在菜单里恢复共享，手机才能打开下面的地址。</span>
          </div>
        )}
        {(step === "cert" ? setup : session) && urls.length === 0 && (
          <p className="text-danger">没有找到可用的局域网地址。请确认电脑已连接 Wi-Fi 或网线。</p>
        )}
        {step === "cert" && setup && current && (
          <CertStep setup={setup} current={current} onIp={setIp} onNext={() => toPairing(false)} onSkip={() => toPairing(true)} />
        )}
        {step === "pair" && session && current && (
          <PairStep session={session} current={current} skipped={skipped} now={now} onIp={setIp} onRefresh={() => void create()} onBack={backToCert} />
        )}
      </div>
    </div>
  );
}

function AddressSelect({ urls, current, onIp }: { urls: PairingUrl[]; current: PairingUrl; onIp: (ip: string) => void }) {
  if (urls.length < 2) return null;
  return (
    <label className="field">
      <span className="field-label">使用的网络地址</span>
      <select className="input" value={current.ip} onChange={(e) => onIp(e.target.value)}>
        {urls.map((u) => (
          <option key={u.url} value={u.ip}>
            {u.ip}（{u.iface}）
          </option>
        ))}
      </select>
    </label>
  );
}

function Qr({ url, alt, expired, children }: { url: PairingUrl; alt: string; expired?: boolean; children?: ReactNode }) {
  return (
    <div className={`qr ${expired ? "is-expired" : ""}`}>
      <img src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(url.qr_svg)}`} alt={alt} />
      {children}
    </div>
  );
}

function CertStep(props: { setup: CertSetup; current: PairingUrl; onIp: (ip: string) => void; onNext: () => void; onSkip: () => void }) {
  const { setup, current } = props;
  return (
    <div className="add-device-body">
      <Qr url={current} alt="安装证书的二维码" />
      <div className="add-device-info">
        <h3 className="add-device-step">第一步：在 iPhone 上安装这台电脑的证书（推荐）</h3>
        <p className="muted small">装好后，Safari 会核对连上的确实是这台电脑，同一网络里的其他设备没法冒充它。</p>
        <ol className="steps">
          <li>iPhone 和这台电脑连同一个 Wi-Fi，用相机扫描二维码，在 Safari 中打开</li>
          <li>
            还没配对过、或者需要重新配对的 iPhone 打开时，Safari 会先提示「此连接非私人连接」。这个页面只提供证书，这时可以在这一页继续访问；真正防冒充的是下一步核对指纹。正在正常使用的已配对 iPhone，请改在手机上 LanShare 右上角菜单里点「安装电脑的证书」
          </li>
          <li>按手机页面的步骤下载证书。安装前，核对证书详情里的 SHA-256 和下面完全一致，不一致就不要安装</li>
          <li>
            在「设置 › 通用 › 关于本机 › 证书信任设置」里打开「<b>{setup.ca_name}</b>」
          </li>
        </ol>
        <div className="fingerprint-box">
          <span className="muted small">本机根证书的 SHA-256：</span>
          <code className="fingerprint-full">
            {fingerprintRows(setup.ca_fingerprint).map((row, i) => (
              <span key={i}>{row}</span>
            ))}
          </code>
        </div>
        <AddressSelect urls={setup.urls} current={current} onIp={props.onIp} />
        <div className="add-device-actions">
          <button className="btn btn-primary" onClick={props.onNext}>
            已装好，下一步：配对
          </button>
          <button className="link-btn link-muted" onClick={props.onSkip}>
            跳过，直接配对（只在你信任的 Wi-Fi 上）
          </button>
        </div>
      </div>
    </div>
  );
}

function PairStep(props: {
  session: PairingSession;
  current: PairingUrl;
  skipped: boolean;
  now: number;
  onIp: (ip: string) => void;
  onRefresh: () => void;
  onBack: () => void;
}) {
  const { session, current } = props;
  const remaining = Math.max(0, Math.ceil((session.expires_at - props.now) / 1000));
  const expired = remaining === 0;
  const origin = new URL(current.url).origin;
  return (
    <div className="add-device-body">
      <Qr url={current} alt="配对二维码" expired={expired}>
        {expired && (
          <button className="btn btn-primary qr-refresh" onClick={props.onRefresh}>
            <Icon name="refresh" size={16} /> 重新生成
          </button>
        )}
      </Qr>
      <div className="add-device-info">
        <h3 className="add-device-step">第二步：配对</h3>
        <ol className="steps">
          <li>用 iPhone 相机扫描二维码，在 Safari 中打开</li>
          <li>在手机上点「请求连接」，然后在这里核对确认码并允许</li>
        </ol>
        {props.skipped && (
          <p className="muted small">
            跳过了证书：Safari 会提示这个连接不是私人连接，只在你信任的 Wi-Fi 上继续。配对后可以在手机右上角菜单里补装证书。
          </p>
        )}
        <div className="pair-code-box">
          <span className="muted small">扫不了码？在手机 Safari 打开下面的地址，输入配对码</span>
          <div className="pair-origin">
            <code>{origin}</code>
            <CopyButton text={origin} />
          </div>
          <div className="pair-code">{session.code.replace(/(\d{3})(\d{3})/, "$1 $2")}</div>
        </div>
        <AddressSelect urls={session.urls} current={current} onIp={props.onIp} />
        <p className="muted small">{expired ? "二维码已过期" : `二维码 ${remaining} 秒后过期，只能使用一次`}</p>
        <button className="link-btn link-muted" onClick={props.onBack}>
          ‹ 回到第一步：安装证书
        </button>
      </div>
    </div>
  );
}
