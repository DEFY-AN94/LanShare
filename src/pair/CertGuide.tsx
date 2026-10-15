import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { api } from "../lib/api";

/** Step-by-step guide for trusting the computer's local CA on iPhone. */
export function CertGuide({ compact }: { compact?: boolean }) {
  // Each CA has its own name ("LanShare Local CA (Mac) 2DD31B"), so the guide
  // can point at the right switch when older ones are still installed.
  const [caName, setCaName] = useState<string | null>(null);
  useEffect(() => {
    api.get<{ name: string }>("/api/ca").then(
      (r) => setCaName(r.name),
      () => {},
    );
  }, []);

  return (
    <section className={`card cert-guide ${compact ? "compact" : ""}`}>
      <h3 className="card-title">
        <Icon name="shield" size={18} /> 去掉「不安全」提示（推荐）
      </h3>
      <p className="muted">
        电脑用的是自己生成的证书，所以 Safari 第一次会提示「此连接不是私密连接」。在 iPhone 上安装一次电脑的证书后，提示会消失，别人也无法冒充你的电脑。
      </p>
      <ol className="steps">
        <li>
          点下面的「下载证书」，弹出提示时选择<b>允许</b>，然后关闭提示。
        </li>
        <li>
          打开「设置」，点顶部的<b>「已下载描述文件」</b>。<b>先别安装</b>：点<b>「更多详细信息」</b>› 点证书名称 ›
          拉到最底部的<b>「指纹」</b>，核对 <b>SHA-256</b> 的开头和电脑上「添加设备」窗口（或电脑端「设置 › 安全」）显示的一致。
          <b>不一致就不要安装。</b>
        </li>
        <li>
          一致后点<b>安装</b>（需要输入锁屏密码）。「会更改 iPhone 上的设置」是系统对所有描述文件的通用提示：这个描述文件只添加一个证书，没有其他配置。
        </li>
        <li>
          打开「设置 › 通用 › 关于本机」，拉到最下面进入<b>「证书信任设置」</b>，打开<b>「{caName || "LanShare Local CA"}」</b>的开关。
          列表里有好几个 LanShare Local CA 时，只开名称完全一致的这个；其他的一般是重置证书之前装的旧证书。
        </li>
        <li>回到 Safari 重新打开 LanShare，地址栏不再显示「不安全」。</li>
        <li>
          最后在 Safari 里点分享 › <b>添加到主屏幕</b>。如果以前添加过、图标是灰色的字母，先删掉旧图标再重新添加。
        </li>
      </ol>
      <a className="btn btn-primary" href="/api/ca.crt">
        <Icon name="download" size={16} />
        <span>下载证书</span>
      </a>
      <p className="muted small">
        核对指纹是为了确认证书确实来自你的电脑，而不是同一网络里冒充的人。这个证书只对局域网地址有效，不能用来冒充其他网站；旧证书或不想用了，可以在「设置 › 通用 › VPN 与设备管理」里按名称找到并移除。重置电脑端证书后需要重新安装（新证书名称末尾的编号不同）。
      </p>
    </section>
  );
}
