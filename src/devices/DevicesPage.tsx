import { useState } from "react";
import { useDialog } from "../components/Dialog";
import { Icon } from "../components/Icon";
import { useToast } from "../components/Toast";
import { api } from "../lib/api";
import { formatDayTime, formatRelative } from "../lib/format";
import type { Device } from "../lib/types";
import { AddDeviceModal } from "./AddDeviceModal";
import { useDevices } from "./useDevices";

export function deviceIcon(name: string) {
  return /iphone|ipad|手机|平板|android|安卓/i.test(name) ? "phone" : "laptop";
}

export function DevicesPage() {
  const { devices, reload } = useDevices();
  const [adding, setAdding] = useState(false);
  const dialog = useDialog();
  const toast = useToast();

  const run = async (p: Promise<unknown>, ok?: string) => {
    try {
      await p;
      if (ok) toast(ok, "success");
      await reload();
    } catch (e) {
      toast((e as Error).message, "error");
    }
  };

  const rename = async (d: Device) => {
    const name = await dialog.prompt({ title: "修改设备名称", initial: d.name, confirmText: "保存" });
    if (name && name !== d.name) await run(api.patch(`/api/host/devices/${d.id}`, { name }));
  };

  const remove = async (d: Device) => {
    const ok = await dialog.confirm({
      title: `移除「${d.name}」？`,
      message: "这台设备会立即断开，需要重新扫码配对才能再连接。和它的聊天记录也会被删除（共享文件夹里的文件不受影响）。",
      confirmText: "移除",
      danger: true,
    });
    if (ok) await run(api.del(`/api/host/devices/${d.id}`), "已移除");
  };

  return (
    <div className="page">
      <div className="page-head">
        <h1>设备</h1>
        <button className="btn btn-primary" onClick={() => setAdding(true)}>
          <Icon name="qr" size={16} />
          <span>添加设备</span>
        </button>
      </div>
      {devices && devices.length === 0 && (
        <div className="empty-state">
          <Icon name="phone" size={40} />
          <p>还没有配对的设备</p>
          <button className="btn btn-primary" onClick={() => setAdding(true)}>
            扫码添加 iPhone
          </button>
        </div>
      )}
      <div className="device-list">
        {devices?.map((d) => (
          <div key={d.id} className="card device-card">
            <div className="device-icon">
              <Icon name={deviceIcon(d.name)} size={24} />
              <span className={`dot ${d.online ? "online" : ""}`} />
            </div>
            <div className="device-body">
              <div className="device-name">
                {d.name}
                {d.needs_repair && <span className="device-tag">需要重新配对</span>}
              </div>
              <div className="muted small">
                {d.needs_repair
                  ? "电脑的证书换过了：在这台手机上删掉旧证书，再点「添加设备」重新配对。聊天记录仍可以在这里查看"
                  : `${d.online ? "在线" : `最后活动：${formatRelative(d.last_seen_at)}`} · 配对于 ${formatDayTime(d.created_at)}`}
              </div>
            </div>
            {!d.needs_repair && (
            <label className="perm">
              <span className="sr-only">文件权限</span>
              <select
                className="input input-small"
                value={d.can_write ? "rw" : "ro"}
                onChange={(e) => void run(api.patch(`/api/host/devices/${d.id}`, { can_write: e.target.value === "rw" }))}
              >
                <option value="rw">文件：读写</option>
                <option value="ro">文件：只读</option>
              </select>
            </label>
            )}
            <button className="icon-btn" title="改名" aria-label="改名" onClick={() => void rename(d)}>
              <Icon name="edit" />
            </button>
            <button className="icon-btn text-danger" title="移除" aria-label="移除" onClick={() => void remove(d)}>
              <Icon name="trash" />
            </button>
          </div>
        ))}
      </div>
      {adding && <AddDeviceModal onClose={() => setAdding(false)} />}
    </div>
  );
}
