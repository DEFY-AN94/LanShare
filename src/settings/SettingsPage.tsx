import { useCallback, useEffect, useRef, useState } from "react";
import { useDialog } from "../components/Dialog";
import { Icon } from "../components/Icon";
import { useToast } from "../components/Toast";
import { CopyButton } from "../chat/MessageBubble";
import { api } from "../lib/api";
import { events } from "../lib/events";
import { hostOpen, hostOpenUrl } from "../lib/fileActions";
import { fingerprintRows } from "../lib/format";
import type { HostOverview, LoginItemInfo } from "../lib/types";
import { useDevices } from "../devices/useDevices";
import { acceleratorFromEvent, displayAccelerator, windowsCode } from "./shortcut";

// Only opened in the browser when asked: LanShare itself never contacts the internet.
const RELEASES_URL = "https://github.com/DEFY-AN94/LanShare/releases";

export function SettingsPage() {
  const [info, setInfo] = useState<HostOverview | null>(null);
  const [loginItem, setLoginItem] = useState<LoginItemInfo | null>(null);
  const [recording, setRecording] = useState(false);
  const { devices } = useDevices();
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
    const offs = ["paused", "resync", "network", "shared_root", "settings"].map((name) => events.on(name, () => void load()));
    return () => offs.forEach((off) => off());
  }, [load]);

  // Notifications can be allowed in System Settings while this window stays open.
  useEffect(() => {
    const refresh = () => void api.get<HostOverview>("/api/host/overview").then(setInfo, () => undefined);
    const onVisible = () => document.visibilityState === "visible" && refresh();
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, []);

  // After recording with the keyboard, focus goes back to the button that started it.
  const changeButton = useRef<HTMLButtonElement>(null);
  const refocus = useRef(false);
  useEffect(() => {
    if (!recording && refocus.current) {
      refocus.current = false;
      changeButton.current?.focus();
    }
  }, [recording]);

  // Launch at login can also be changed in System Settings while this window stays open.
  useEffect(() => {
    const refresh = () => void api.get<LoginItemInfo>("/api/host/login-item").then(setLoginItem, () => undefined);
    refresh();
    const onVisible = () => document.visibilityState === "visible" && refresh();
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, []);

  const setLoginItemOn = async (on: boolean) => {
    try {
      setLoginItem(await api.post<LoginItemInfo>("/api/host/login-item", { on }));
    } catch (e) {
      toast((e as Error).message, "error");
    }
  };

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
        "会生成新的根证书和服务器证书，新证书名称末尾的编号会变。所有已配对的设备都要重新配对：它们在设备列表里会标成「需要重新配对」，聊天记录仍可以在这台电脑上查看。在 iPhone 的「设置 › 通用 › VPN 与设备管理」里删掉旧证书，再按「添加设备」第一步装新证书、重新配对。",
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

  const fingerprint = fingerprintRows(info.ca_fingerprint).join(" ");
  const mac = info.platform === "macos";
  const trayName = mac ? "菜单栏" : "任务栏通知区域";
  const notifyStatus: Record<string, string> = {
    allowed: "通知已打开",
    denied: mac ? "LanShare 的通知被关掉了：在「系统设置 › 通知 › LanShare」里打开" : "LanShare 的通知被关掉了：在 Windows「设置 › 系统 › 通知」里打开",
    notDetermined: "还没有允许通知：macOS 问的时候点「允许」",
    unavailable: "这个版本不是从 LanShare.app 运行的，只能显示普通通知（没有按钮）",
  };

  const setShortcut = async (shortcut: string | null) => {
    setRecording(false);
    await run(api.post("/api/host/clipboard/shortcut", { shortcut }), shortcut ? "快捷键已设置" : "快捷键已清除");
  };

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

      {loginItem && loginItem.state !== "unsupported" && (
        <section className="card">
          <h3 className="card-title">启动</h3>
          <div className="setting-row">
            <div>
              <div className="setting-title">
                登录时自动启动
                {loginItem.state === "on" ? "（已打开）" : loginItem.state === "needsApproval" ? (mac ? "（在系统设置里关着）" : "（在启动应用里关着）") : ""}
              </div>
              <div className="muted small">
                {loginItem.state === "on" && `登录${mac ? " Mac" : " Windows"} 后 LanShare 会在${trayName}里运行，不打开窗口，手机随时能连上。`}
                {loginItem.state === "off" && `打开后，登录${mac ? " Mac" : " Windows"} 时 LanShare 会自动在${trayName}里运行，手机随时能连上。`}
                {/* macOS reports an item switched off in System Settings like one still waiting for approval. */}
                {loginItem.state === "needsApproval" &&
                  (mac
                    ? "登录 Mac 时不会自动启动：LanShare 在「系统设置 › 通用 › 登录项」里是关着的。要打开，请在那里允许 LanShare。"
                    : "登录 Windows 时不会自动启动：LanShare 在「设置 › 应用 › 启动」（或任务管理器的「启动应用」）里是关着的。点「打开」可以重新打开。")}
                {loginItem.state === "notInstalled" &&
                  (mac ? "把 LanShare 移到「应用程序」文件夹并从那里打开后，才能设置。" : "用安装程序安装 LanShare，并从开始菜单打开后，才能设置。")}
              </div>
            </div>
            {loginItem.state === "needsApproval" && mac ? (
              <button className="btn" onClick={() => void run(api.post("/api/host/login-item/settings"))}>
                打开系统设置
              </button>
            ) : (
              loginItem.state !== "notInstalled" && (
                <button className={`btn ${loginItem.state === "on" ? "btn-primary" : ""}`} onClick={() => void setLoginItemOn(loginItem.state !== "on")}>
                  {loginItem.state === "on" ? "关闭" : "打开"}
                </button>
              )
            )}
          </div>
        </section>
      )}

      <section className="card">
        <h3 className="card-title">通知</h3>
        {info.notifications !== "unknown" && (
          <div className={info.notifications === "allowed" ? "muted small" : "banner banner-warn banner-block"}>
            {info.notifications !== "allowed" && <Icon name="alert" size={16} />}
            <span>{notifyStatus[info.notifications] ?? ""}</span>
          </div>
        )}
        <div className="setting-row">
          <div>
            <div className="setting-title">通知里不显示消息内容{info.notify_hide_text ? "（已打开）" : ""}</div>
            <div className="muted small">只显示是哪台设备发来的。「复制」按钮仍然复制完整的文字。</div>
          </div>
          <button
            className={`btn ${info.notify_hide_text ? "btn-primary" : ""}`}
            onClick={() => void run(api.post("/api/host/notifications", { hide_text: !info.notify_hide_text }))}
          >
            {info.notify_hide_text ? "关闭" : "打开"}
          </button>
        </div>
        <p className="muted small">
          手机发来文字时，通知上的「复制」会复制全文；发来文件时，「打开」会用默认程序打开（程序、脚本、安装包、磁盘映像、描述文件和证书只在文件夹中显示）。点通知会打开这段对话。同一台设备连续发来很多条时，只弹出前两条，后面的合并显示。
        </p>
      </section>

      <section className="card">
        <h3 className="card-title">剪贴板</h3>
        <p className="muted small">
          在{trayName}的 LanShare 图标菜单里选「发送剪贴板到…」，把电脑上复制的文字或截图发到手机；「复制最近收到的文字」把手机发来的最新一段文字放进剪贴板。只有这时才会读取剪贴板。
        </p>
        <div className="setting-row">
          <div>
            <div className="setting-title">默认发送给</div>
            <div className="muted small">快捷键和菜单里的「发送剪贴板到…」发给这台设备。</div>
          </div>
          <label>
            <span className="sr-only">默认发送给</span>
            <select
              className="input input-small"
              value={info.clipboard_target ?? ""}
              onChange={(e) => void run(api.post("/api/host/clipboard/target", { device: e.target.value || null }))}
            >
              <option value="">最近聊天的设备</option>
              {(devices ?? []).filter((d) => !d.needs_repair).map((d) => (
                <option key={d.id} value={d.id}>
                  {d.name}
                </option>
              ))}
            </select>
          </label>
        </div>
        {info.clipboard_shortcut_error && (
          <div className="banner banner-warn banner-block">
            <Icon name="alert" size={16} />
            <span>LanShare 启动时没能设置快捷键：{info.clipboard_shortcut_error}</span>
          </div>
        )}
        <div className="setting-row">
          <div>
            <div className="setting-title">
              全局快捷键：{info.clipboard_shortcut ? <kbd>{displayAccelerator(info.clipboard_shortcut, info.platform)}</kbd> : "未设置"}
            </div>
            <div className="muted small">
              {recording
                ? mac
                  ? "现在按下想用的组合键（至少两个修饰键，其中要有 ⌃ 或 ⌘，例如 ⌃⌥⌘V），按 Esc 取消。"
                  : "现在按下想用的组合键（至少两个修饰键，其中要有 Ctrl 或 Win，再加一个字母、数字、F1–F12 或空格，例如 Ctrl+Shift+F9；Ctrl+Alt 等同于 AltGr，会挡住打字，不能用），按 Esc 取消。"
                : "在任何 App 里按下就把剪贴板发出去。其他 App 自己的快捷键检测不到，按了没反应就换一个组合。"}
            </div>
          </div>
          <div className="row-actions">
            {recording ? (
              <ShortcutRecorder
                windows={info.platform === "windows"}
                onDone={(acc, byKeyboard) => {
                  refocus.current = byKeyboard;
                  if (acc) void setShortcut(acc);
                  else setRecording(false);
                }}
              />
            ) : (
              <button ref={changeButton} className="btn" onClick={() => setRecording(true)}>
                {info.clipboard_shortcut ? "更改…" : "设置…"}
              </button>
            )}
            {info.clipboard_shortcut && !recording && (
              <button className="btn" onClick={() => void setShortcut(null)}>
                清除
              </button>
            )}
          </div>
        </div>
      </section>

      <section className="card">
        <h3 className="card-title">共享文件夹</h3>
        {info.preferred_root && (
          <div className="banner banner-warn banner-block">
            <Icon name="alert" size={16} />
            <span>
              选好的共享文件夹 <code>{info.preferred_root}</code> 在 LanShare 启动时不可用（所在的磁盘可能没有连接），暂时使用下面这个文件夹。重新连接后会自动换回去。
            </span>
          </div>
        )}
        {info.shared_root_unavailable && (
          <div className="banner banner-warn banner-block">
            <Icon name="alert" size={16} />
            <span>
              {info.shared_root_unavailable === "disk"
                ? "这个文件夹现在不可用，所在的磁盘可能没有连接，手机暂时无法发送和浏览文件。重新连接磁盘后会自动恢复，也可以点「更改…」换一个文件夹。"
                : "这个文件夹不见了，可能被移动、改名或删除了，手机暂时无法发送和浏览文件。放回原处后会自动恢复，也可以点「更改…」换一个文件夹。"}
            </span>
          </div>
        )}
        <div className="setting-row">
          <code className="path ellipsis" title={info.shared_root}>
            {info.shared_root}
          </code>
          <div className="row-actions">
            <button className="btn" disabled={!!info.shared_root_unavailable} onClick={() => void run(hostOpen(""))}>
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
          连不上时检查：手机和电脑是否在同一个 Wi-Fi；电脑是否已睡眠（传输文件时 LanShare 会让电脑保持唤醒）；访客网络、公司网络可能禁止设备互访；手机上的
          VPN 可能拦截局域网流量；macOS 防火墙里要允许 LanShare 接受传入连接（Windows 上还要把当前网络设为「专用网络」）。
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
          <div className="muted small">怀疑证书泄露或想重新开始时可以重置。重置后所有设备都要重新配对。</div>
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

/**
 * Listens for the next key combination; Esc (or leaving the field) cancels.
 * Meanwhile the current shortcut sends nothing: the system hands it to
 * LanShare rather than to this page, and pressing it ends the recording.
 */
function ShortcutRecorder({ windows, onDone }: { windows: boolean; onDone: (accelerator: string | null, byKeyboard: boolean) => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  const done = useRef(onDone);
  done.current = onDone;
  // Once only: the button can still lose focus as it goes away.
  const finished = useRef(false);
  const finish = (accelerator: string | null, byKeyboard: boolean) => {
    if (finished.current) return;
    finished.current = true;
    done.current(accelerator, byKeyboard);
  };
  const finishRef = useRef(finish);
  finishRef.current = finish;
  const toast = useToast();
  useEffect(() => {
    ref.current?.focus();
    const recording = (on: boolean) => void api.post("/api/host/clipboard/shortcut/recording", { on }).catch(() => undefined);
    recording(true);
    // The computer forgets a recording after two minutes (in case this page goes away).
    const renew = window.setInterval(() => recording(true), 30_000);
    const off = events.on("shortcut_pressed", () => {
      toast("这就是现在的快捷键");
      finishRef.current(null, true);
    });
    return () => {
      window.clearInterval(renew);
      off();
      recording(false);
    };
  }, [toast]);
  return (
    <button
      ref={ref}
      className="btn btn-primary"
      onBlur={() => finish(null, false)}
      onKeyDown={(e) => {
        e.preventDefault();
        if (e.key === "Escape") return finish(null, true);
        // A Mac registers the key where it sits (e.code); Windows by virtual key.
        const code = windows ? windowsCode(e.keyCode) : e.code;
        if (code === null) return;
        const acc = acceleratorFromEvent({ code, ctrlKey: e.ctrlKey, altKey: e.altKey, shiftKey: e.shiftKey, metaKey: e.metaKey }, windows);
        if (acc) finish(acc, true);
      }}
    >
      请按组合键…
    </button>
  );
}
