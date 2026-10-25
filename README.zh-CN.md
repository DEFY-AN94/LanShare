# LanShare

<p>
  <a href="README.md"><img alt="English" src="https://img.shields.io/badge/English-6e7781?style=for-the-badge"></a>
  <a href="README.zh-CN.md"><img alt="简体中文" src="https://img.shields.io/badge/%E7%AE%80%E4%BD%93%E4%B8%AD%E6%96%87-0969da?style=for-the-badge"></a>
</p>

局域网内 iPhone ↔ 电脑的消息与文件传输工具，只走局域网，不经过任何云端。完整设计见 [PLAN.md](PLAN.md)（英文）。

- **消息**：类 iMessage 聊天，每条文字消息旁有快速复制按钮；可直接发图片和文件
- **文件**：iPhone 浏览电脑上的共享文件夹（默认 `~/LanShare`），上传、下载、预览、重命名、删除（进废纸篓 / 回收站）
- **iPhone 不用装 App**：Safari 扫码打开即可，可以添加到主屏幕
- **安全**：局域网通信全程 HTTPS；扫码配对需要电脑端确认；只接受私有网络地址的连接（Windows 上安装程序的防火墙规则进一步限定为本子网）；访问范围限制在共享文件夹内

当前进度：Mac 版已完成；Windows 版由 GitHub Actions 编译、测试并打包（PLAN.md 里程碑 M6）。

## 使用

1. 打开 LanShare（在仓库的 **Releases** 页面下载，或者按下面「开发」一节自己构建）。第一次运行时 macOS 可能会问「是否允许 LanShare 接受传入的网络连接」，要点**允许**，否则手机连不上。
2. 在电脑端点「添加设备」（iPhone 和电脑要连同一个 Wi-Fi）。
3. 第一步（推荐）：用 iPhone 相机扫第一个二维码，按步骤安装电脑的证书。安装前核对证书的 SHA-256 指纹和电脑上显示的完全一致。装好证书后，Safari 会核对连上的确实是你的电脑。在你信任的 Wi-Fi 上也可以跳过这一步，这时 Safari 会提示连接不是私人连接，安全就只能靠网络本身。
4. 第二步：扫配对二维码，在 iPhone 上点「请求连接」，核对两边的确认码一致后，在电脑上点「允许」。
5. 在 Safari 里点分享 › 添加到主屏幕，以后一点就开。以前添加的图标如果是灰色字母，删掉后重新添加即可。

从 iPhone 的「照片图库」上传时，iOS 会自动把 HEIC 照片转成 JPEG、HEVC 视频转成 H.264（分辨率不变）。需要原片时，先在「照片」App 里分享 › 存储到「文件」，再在 LanShare 里选「选取文件」。第一次上传时页面会提示一次。

关掉窗口后 LanShare 会继续在后台运行：macOS 上在菜单栏和 Dock，Windows 上在任务栏通知区域（系统托盘）。菜单栏 / 托盘图标的菜单里可以「打开 LanShare」、「暂停共享」或「退出 LanShare」。

装好并信任了电脑证书的 iPhone 打开 LanShare 时不会出现证书警告；如果出现了，请关掉页面。在设置里「重置证书」后，所有手机都要重新配对：先在 iPhone 上删掉旧证书，再走一遍「添加设备」。主屏幕上的图标会保留添加时的登录，所以也要删掉，配对后再从 Safari 重新添加。旧的设备在列表里标成「需要重新配对」，聊天记录仍可以在电脑上查看。

电脑连不上时（电脑睡眠了，或 iPhone 不在同一个网络），主屏幕上的 App 照样能打开，并会自己不断重连。这期间发出的文字先存在 iPhone 上，连上后自动送达；没发出的草稿关掉 App 也不会丢。点按一条消息可以复制或删除，删除后两边都会消失。删除文件消息时，文件默认留在电脑上，也可以选择把它移到废纸篓。

上传进行中，iPhone 的屏幕不会熄灭，电脑也不会睡眠。如果上传后电脑剩余空间会太少，上传会被拒绝，iPhone 上会显示具体数字：LanShare 总会给磁盘留出 5% 的空间，至少 2 GB，最多 20 GB。

设置 ›「启动」可以让 LanShare 在登录 Mac（或 Windows）时自动启动，只显示在菜单栏或任务栏通知区域。默认关闭，第一次配对成功后会提示一次。macOS 会把它列在「系统设置 › 通用 › 登录项」里。

手机发来消息时的通知带按钮：「复制」复制完整的文字（不只是预览），「打开」打开文件（程序、脚本、安装包、磁盘映像、描述文件和证书只在文件夹中显示）。点通知会打开这段对话。同一台手机连续发来很多条时，只弹出前两条，后面的合并显示。未读数显示在 Dock 图标上（LanShare 没有 Dock 图标时显示在菜单栏图标旁边；Windows 上显示在托盘图标的提示里）。设置 ›「通知」可以让通知里不显示消息内容。

菜单栏 / 托盘菜单也能反过来用：「发送剪贴板到…」把电脑上复制的文字、截图或图片发到手机（发给最近聊天的设备，或你选的设备）；「复制最近收到的文字」把手机发来的最新一段文字放进剪贴板。设置 ›「剪贴板」可以给发送加一个全局快捷键（至少两个修饰键，其中要有 ⌃ 或 ⌘，例如 ⌃⌥⌘V），默认没有。只有你这样操作时，LanShare 才会读取剪贴板。

LanShare 不会自己检查更新。设置 ›「查看新版本」会用浏览器打开 **Releases** 页面，每个版本都写了改动，并附有两个安装包和校验和。

## Windows 版

安装包 `LanShare_<版本>_x64-setup.exe` 在仓库的 **Releases** 页面下载。GitHub Actions（[`.github/workflows/ci.yml`](.github/workflows/ci.yml)）也会在 Windows 上跑全部测试，并把每次构建的安装包作为构建产物保存：Actions › 某次成功的运行 › Artifacts › `LanShare-windows-x64`（解压后是 `.exe`，构建产物保留 90 天）。两种方式都需要登录有这个仓库权限的 GitHub 账号。Windows 版还没有在真机上手动测试过（见 PLAN.md 的「Windows testing, later」）。

- 安装包需要管理员权限（装到「Program Files」），安装时会自动添加防火墙规则：只允许同一子网的设备、只在「专用网络」下访问 LanShare；卸载时删除规则
- 如果 Windows 把当前 Wi-Fi 设成了「公用网络」，手机连不上，LanShare 的设置页和添加设备窗口会提示改成「专用网络」
- 安装包没有代码签名，首次运行 SmartScreen 会拦截：点「更多信息 › 仍要运行」
- 共享文件夹在 U 盘或网络共享上时，LanShare 不能删除那里的文件，手机端和电脑窗口里都不行（那里没有回收站，删除就是永久删除），请在文件资源管理器里删除

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
cargo test --manifest-path src-tauri/Cargo.toml   # Rust 单元测试
npx tsc                                           # 前端类型检查
```

接口端到端测试会配对一台假手机，所以要单独起一个临时的 headless 服务。脚本从 `.dev-data/headless.log` 读取服务的登录地址；局域网端口默认 39210，被占用时（比如 LanShare 本身在运行）会换成别的，所以端口也从日志里取：

```bash
mkdir -p .dev-data
npm run headless -- --data .dev-data/smoke --shared .dev-data/smoke-share --local-port 53299 --upload-reserve-mb 0 > .dev-data/headless.log 2>&1 &
# 等日志里出现 "Press Ctrl+C"（第一次编译要等一会儿），再运行：
PORT=$(grep -o 'listening on port [0-9]*' .dev-data/headless.log | grep -o '[0-9]*$')
PHONE_OVERRIDE=https://127.0.0.1:$PORT scripts/api_smoke.sh .
```

### 发布

在 `main` 上推送版本标签后，GitHub Actions 会自动构建、检查并发布新版本，附两个安装包和校验和。步骤见 [docs/releasing.md](docs/releasing.md)（英文）。

## 目录

```
src/                网页界面（电脑窗口和 iPhone 共用）
  app/ chat/ files/ devices/ settings/ pair/ components/ lib/
src-tauri/src/
  lib.rs            桌面外壳：窗口、托盘图标、通知、选择文件夹
  core/             服务端：证书、配对、鉴权、消息、文件、SSE、数据库
src-tauri/examples/headless.rs   无窗口运行服务
src-tauri/windows/hooks.nsh      Windows 安装包钩子（防火墙规则）
scripts/api_smoke.sh             接口端到端测试
scripts/iphone_check.sh          iPhone 15 分钟检查的测试页（docs/iphone-check.md）
scripts/release_check.sh         检查发布标签和代码是否一致（docs/releasing.md）
.github/workflows/ci.yml         CI：Windows 测试和安装包
.github/workflows/release.yml    发布：推送标签 vX.Y.Z 后发布两个安装包
```

## 数据位置

- 应用数据（数据库、证书、配置）：macOS 在 `~/Library/Application Support/com.lanshare.desktop/`，Windows 在 `%APPDATA%\com.lanshare.desktop\`
- 共享文件夹：默认 `~/LanShare`（Windows 是 `%USERPROFILE%\LanShare`），聊天里收到的文件在「收件箱」，发给手机的在「已发送」（这两个文件夹只在电脑上可见）
- 日志：应用数据里的 `logs` 文件夹（设置 ›「打开日志文件夹」），最多约 10 MB，不会记录消息内容和文件名；排查问题时可以在设置里打开「详细日志」，重启 LanShare 后自动关闭
- 聊天记录可以设为 30 天或 90 天后自动删除（设置 ›「自动删除旧聊天记录」）；删除的消息会从数据库文件里彻底清除，而不只是隐藏
- 局域网端口：默认 39210（被占用时自动换一个，并记住）

## 已知限制

- iPhone 切到后台或锁屏后连接会断开，重新打开时自动补齐消息；纯局域网下没有推送通知
- 上传中途锁屏：实测 600 MB 视频锁屏后仍传完，但锁屏时间长了 iOS 可能会中断上传，目前还不支持断点续传
- 公司、酒店、访客 Wi-Fi 常开启「AP 隔离」，设备之间无法互访
- 电脑 IP 变化后，旧的主屏幕书签会失效，重新扫码即可
- 应用未签名：在别的 Mac 上首次打开需要到「系统设置 › 隐私与安全性」里点「仍要打开」

## 许可证

LanShare 以 [MIT 许可证](LICENSE) 发布。
