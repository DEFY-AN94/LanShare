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
          打开「设置」，点顶部的<b>「已下载描述文件」</b>，然后点<b>安装</b>（需要输入锁屏密码）。
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
      <p className="muted small">这个证书只对局域网地址有效，不能用来冒充其他网站。重置电脑端证书后需要重新安装。</p>
    </section>
  );
}
