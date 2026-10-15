# LanShare

局域网内 iPhone ↔ 电脑的消息与文件传输工具，只走局域网，不经过任何云端。完整设计见 [PLAN.md](PLAN.md)。

- **消息**：类 iMessage 聊天，每条消息旁有快速复制按钮；可直接发图片和文件
- **文件**：iPhone 浏览电脑上的共享文件夹（默认 `~/LanShare`），上传、下载、预览、重命名、删除（进回收站）
- **iPhone 不用装 App**：Safari 扫码打开即可，可以添加到主屏幕
- **安全**：全程 HTTPS；扫码配对需要电脑端确认；只接受局域网连接；访问范围限制在共享文件夹内

当前进度：Mac 版 MVP。Windows 版等 GitHub 仓库建好后用 GitHub Actions 打包（PLAN.md 里程碑 M6）。

## 使用

1. 打开 LanShare。第一次运行时 macOS 可能会问「是否允许 LanShare 接受传入的网络连接」，要点**允许**，否则手机连不上。
2. 在电脑端点「添加设备」，用 iPhone 相机扫二维码（iPhone 和电脑要连同一个 Wi-Fi）。
3. Safari 提示「此连接不是私密连接」时，点 显示详细信息 › 访问此网站。想彻底去掉这个提示，配对后按页面引导安装电脑的证书（推荐），安装前核对证书指纹和电脑上显示的一致。
4. 在 iPhone 上点「请求连接」，核对两边的确认码一致后，在电脑上点「允许」。
5. 在 Safari 里点分享 › 添加到主屏幕，以后一点就开。

关掉窗口后 LanShare 会继续在后台运行（菜单栏图标 / Dock），可以从菜单栏图标里暂停共享或退出。

## 开发

需要：Xcode 命令行工具、Rust（`brew install rust`）、Node.js 22+。

```bash
npm install
npm run build          # 构建网页界面到 dist/（Rust 端从这里读取）
npm run tauri dev      # 运行桌面版（调试）
npx tauri build        # 打包 .app / .dmg，输出在 src-tauri/target/release/bundle/
```

不开窗口、只跑服务（数据放在 `.dev-data/`，不影响正式数据）：

```bash
npm run headless -- --local-port 53299
```

终端会打印手机访问地址和电脑端界面的登录地址。改了前端后重新 `npm run build` 并刷新页面即可，调试版服务直接从 `dist/` 读文件。

### 测试

```bash
cargo test --manifest-path src-tauri/Cargo.toml                       # Rust 单元测试
PHONE_OVERRIDE=https://127.0.0.1:39210 scripts/api_smoke.sh .         # 接口端到端测试（需先运行 headless）
npx tsc                                                               # 前端类型检查
```

## 目录

```
src/                网页界面（电脑窗口和 iPhone 共用）
  app/ chat/ files/ devices/ settings/ pair/ components/ lib/
src-tauri/src/
  lib.rs            桌面外壳：窗口、菜单栏图标、通知、选择文件夹
  core/             服务端：证书、配对、鉴权、消息、文件、SSE、数据库
src-tauri/examples/headless.rs   无窗口运行服务
scripts/api_smoke.sh             接口端到端测试
```

## 数据位置

- 应用数据（数据库、证书、配置）：`~/Library/Application Support/com.lanshare.desktop/`
- 共享文件夹：默认 `~/LanShare`，聊天里收到的文件在「收件箱」，发给手机的在「已发送」（这两个文件夹只在电脑上可见）
- 局域网端口：默认 39210（被占用时自动换一个，并记住）

## 已知限制

- iPhone 切到后台或锁屏后连接会断开，重新打开时自动补齐消息；纯局域网下没有推送通知
- 公司、酒店、访客 Wi-Fi 常开启「AP 隔离」，设备之间无法互访
- 电脑 IP 变化后，旧的主屏幕书签会失效，重新扫码即可
- 应用未签名：在别的 Mac 上首次打开需要到「系统设置 › 隐私与安全性」里点「仍要打开」
