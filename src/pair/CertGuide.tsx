import { Icon } from "../components/Icon";

/** Step-by-step guide for trusting the computer's local CA on iPhone. */
export function CertGuide({ compact }: { compact?: boolean }) {
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
          点下面的「下载证书」，弹出提示时选择<b>允许</b>。
        </li>
        <li>
          打开「设置」，点顶部的<b>「已下载描述文件」</b>。安装前先点<b>「更多详细信息」</b>，进入证书，核对
          <b>SHA-256</b> 的开头和电脑上「添加设备」窗口（或「设置 › 安全」）里显示的一致——<b>不一致就不要安装</b>。
        </li>
        <li>
          确认一致后点<b>安装</b>（需要输入锁屏密码）。
        </li>
        <li>
          打开「设置 › 通用 › 关于本机」，拉到最下面进入<b>「证书信任设置」</b>，打开「LanShare Local CA」的开关。
        </li>
        <li>回到 Safari 重新打开 LanShare，地址栏不再显示「不安全」。</li>
      </ol>
      <a className="btn btn-primary" href="/api/ca.crt">
        <Icon name="download" size={16} />
        <span>下载证书</span>
      </a>
      <p className="muted small">
        核对指纹是为了确认证书确实来自你的电脑，而不是同一网络里冒充的人。电脑生成的证书只对局域网地址有效；重置电脑端证书后需要重新安装。
      </p>
    </section>
  );
}
