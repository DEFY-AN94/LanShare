import { useCallback, useEffect, useState } from "react";
import { useDialog } from "../components/Dialog";
import { Icon } from "../components/Icon";
import { useToast } from "../components/Toast";
import { CopyButton } from "../chat/MessageBubble";
import { api } from "../lib/api";
import { events } from "../lib/events";
import { hostOpen, hostOpenUrl } from "../lib/fileActions";
import type { HostOverview } from "../lib/types";

// Only opened in the browser when asked: LanShare itself never contacts the internet.
const RELEASES_URL = "https://github.com/DEFY-AN94/LanShare/releases";

export function SettingsPage() {
  const [info, setInfo] = useState<HostOverview | null>(null);
  const dialog = useDialog();
  const toast = useToast();

  const load = useCallback(async () => {
    try {
      setInfo(await api.get<HostOverview>("/api/host/overview"));
    } catch (e) {
      toast((e as Error).message, "error");
    }
  }, [toast]);

  useEffect(() => {
    void load();
    const offs = ["paused", "resync", "network"].map((name) => events.on(name, () => void load()));
    return () => offs.forEach((off) => off());
  }, [load]);

  if (!info) return <div className="page">正在读取…</div>;

  const run = async (p: Promise<unknown>, ok?: string) => {
    try {
      await p;
      if (ok) toast(ok, "success");
    } catch (e) {
      toast((e as Error).message, "error");
    } finally {
      // Show what the computer really has, also after a failure.
      await load();
    }
  };

  const pickFolder = async () => {
    try {
      const next = await api.post<HostOverview | undefined>("/api/host/shared-root/pick");
      if (next) {
        setInfo(next);
        toast("共享文件夹已更改", "success");
      }
    } catch (e) {
      toast((e as Error).message, "error");
    }
  };

  const resetCert = async () => {
    const ok = await dialog.confirm({
      title: "重置证书？",
      message:
        "会生成新的根证书和服务器证书，新证书名称末尾的编号会变。已在 iPhone 上安装旧证书的话，需要在「VPN 与设备管理」里删除旧证书并重新安装；已配对的设备不受影响。",
      confirmText: "重置",
      danger: true,
    });
    if (ok) await run(api.post("/api/host/reset-cert"), "证书已重置");
  };

  const clearMessages = async () => {
    const ok = await dialog.confirm({
      title: "清空所有聊天记录？",
      message: "所有设备的聊天记录都会被删除，无法恢复。聊天里收发的文件仍保留在共享文件夹中。",
      confirmText: "清空",
      danger: true,
    });
    if (ok) await run(api.post("/api/host/clear-messages"), "聊天记录已清空");
  };

  const setRetention = async (days: number) => {
    if (days > 0) {
      const ok = await dialog.confirm({
        title: `只保留最近 ${days} 天的聊天记录？`,
        message: `超过 ${days} 天的消息会立即从这台电脑上删除，无法恢复；以后每 6 小时检查一次。聊天里收发的文件仍保留在共享文件夹中。`,
        confirmText: "确定",
        danger: true,
      });
      if (!ok) return;
    }
    await run(api.post("/api/host/chat-retention", { days }), days > 0 ? `已设为保留 ${days} 天` : "已关闭自动删除");
  };

  const fingerprint = info.ca_fingerprint.match(/.{1,2}/g)?.join(":").toUpperCase() ?? "";

  return (
    <div className="page settings">
      <h1>设置</h1>

      <section className="card">
        <div className="setting-row">
          <div>
            <div className="setting-title">{info.paused ? "共享已暂停" : "共享已开启"}</div>
            <div className="muted small">{info.paused ? "手机等设备暂时无法访问这台电脑" : "已配对的设备可以在局域网内访问"}</div>
          </div>
          <button className={`btn ${info.paused ? "btn-primary" : ""}`} onClick={() => void run(api.post("/api/host/paused", { paused: !info.paused }))}>
            <Icon name={info.paused ? "play" : "pause"} size={16} />
            <span>{info.paused ? "恢复共享" : "暂停共享"}</span>
          </button>
        </div>
      </section>

      <section className="card">
        <h3 className="card-title">共享文件夹</h3>
        <div className="setting-row">
          <code className="path ellipsis" title={info.shared_root}>
            {info.shared_root}
          </code>
          <div className="row-actions">
            <button className="btn" onClick={() => void run(hostOpen(""))}>
              打开
            </button>
            <button className="btn" onClick={() => void pickFolder()}>
              更改…
            </button>
          </div>
        </div>
        <p className="muted small">
          手机只能看到这个文件夹里的内容。聊天中收到的文件放在「收件箱」，发给手机的文件放在「已发送」；这两个文件夹只在电脑上可见，手机只能通过聊天消息打开各自的文件。
        </p>
      </section>

      <section className="card">
        <h3 className="card-title">网络</h3>
        {info.network_warning && (
          <div className="banner banner-warn banner-block">
            <Icon name="alert" size={16} />
            <span>{info.network_warning}</span>
          </div>
        )}
        {info.addresses.length === 0 && <p className="text-danger">没有找到局域网地址，请检查网络连接。</p>}
        {info.addresses.map((a) => {
          const url = `https://${a.ip}:${info.port}`;
          return (
            <div key={a.ip} className="setting-row">
              <div>
                <code>{url}</code>
                <div className="muted small">{a.iface}</div>
              </div>
              <CopyButton text={url} />
            </div>
          );
        })}
        <p className="muted small">
          连不上时检查：手机和电脑是否在同一个 Wi-Fi；访客网络、公司网络可能禁止设备互访；手机上的 VPN 可能拦截局域网流量；macOS
          防火墙里要允许 LanShare 接受传入连接（Windows 上还要把当前网络设为「专用网络」）。
        </p>
      </section>

      <section className="card">
        <h3 className="card-title">安全</h3>
        <div className="setting-row">
          <div>
            <div className="setting-title">本机根证书名称</div>
            <code>{info.ca_name}</code>
            <div className="muted small">
              iPhone 的「证书信任设置」和「VPN 与设备管理」按这个名称列出证书。看到其他名称的 LanShare Local CA，一般是重置之前装的旧证书，可以删除。
            </div>
          </div>
        </div>
        <div className="setting-row">
          <div>
            <div className="setting-title">本机根证书指纹（SHA-256）</div>
            <code className="fingerprint">{fingerprint}</code>
            <div className="muted small">在 iPhone 上安装证书前，核对证书详情里的 SHA-256 与此一致。</div>
          </div>
        </div>
        <div className="setting-row">
          <div className="muted small">怀疑证书泄露或想重新开始时可以重置。</div>
          <button className="btn btn-danger-outline" onClick={() => void resetCert()}>
            重置证书
          </button>
        </div>
      </section>

      <section className="card">
        <h3 className="card-title">聊天记录</h3>
        <div className="setting-row">
          <div>
            <div className="setting-title">自动删除旧聊天记录</div>
            <div className="muted small">超过所选时间的消息会自动删除。聊天里收发的文件仍保留在共享文件夹中。</div>
          </div>
          <label>
            <span className="sr-only">自动删除旧聊天记录</span>
            <select className="input input-small" value={info.chat_retention_days} onChange={(e) => void setRetention(Number(e.target.value))}>
              <option value={0}>不自动删除</option>
              <option value={30}>保留 30 天</option>
              <option value={90}>保留 90 天</option>
            </select>
          </label>
        </div>
        <div className="setting-row">
          <div className="muted small">聊天记录只保存在这台电脑上。</div>
          <button className="btn btn-danger-outline" onClick={() => void clearMessages()}>
            清空所有聊天记录
          </button>
        </div>
      </section>

      <section className="card">
        <h3 className="card-title">日志</h3>
        <div className="setting-row">
          <div>
            <div className="setting-title">详细日志{info.verbose_logging ? "（已打开）" : ""}</div>
            <div className="muted small">排查连接问题时打开，会多记录一些细节，例如证书握手失败。重启 LanShare 后自动关闭。</div>
          </div>
          <button
            className={`btn ${info.verbose_logging ? "btn-primary" : ""}`}
            onClick={() => void run(api.post("/api/host/logs/verbose", { on: !info.verbose_logging }))}
          >
            {info.verbose_logging ? "关闭" : "打开"}
          </button>
        </div>
        <div className="setting-row">
          <div className="muted small">日志只保存在这台电脑上，最多约 10 MB，不会记录消息内容和文件名。</div>
          <button className="btn" onClick={() => void run(api.post("/api/host/logs/open"))}>
            打开日志文件夹
          </button>
        </div>
      </section>

      <p className="muted small center">
        LanShare {info.version} · {info.host_name} ·{" "}
        <a
          href={RELEASES_URL}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(e) => {
            // The desktop window must never navigate away; open in the system browser.
            e.preventDefault();
            void hostOpenUrl(RELEASES_URL).catch((err: Error) => toast(err.message, "error"));
          }}
        >
          查看新版本
        </a>
      </p>
    </div>
  );
}
