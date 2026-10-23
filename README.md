# LanShare

<p>
  <a href="README.md"><img alt="English" src="https://img.shields.io/badge/English-0969da?style=for-the-badge"></a>
  <a href="README.zh-CN.md"><img alt="简体中文" src="https://img.shields.io/badge/%E7%AE%80%E4%BD%93%E4%B8%AD%E6%96%87-6e7781?style=for-the-badge"></a>
</p>

Send messages and files between an iPhone and a computer over your local network. Everything stays on the LAN and nothing goes through the cloud. The full design is in [PLAN.md](PLAN.md).

- **Messages**: an iMessage-style chat with a quick copy button next to every text message. Photos and files can be sent straight from the chat.
- **Files**: the iPhone browses a shared folder on the computer (`~/LanShare` by default) and can upload, download, preview, rename and delete (deleted files go to the Trash or Recycle Bin).
- **No iPhone app to install**: scan a QR code to open LanShare in Safari, and add it to the Home Screen if you like.
- **Secure**: all network traffic is HTTPS; every pairing must be approved on the computer; only connections from the local network are accepted; phones can only reach the shared folder.

Status: the Mac version is done. The Windows version is built, tested and packaged by GitHub Actions (milestone M6 in PLAN.md).

> LanShare's interface is currently in Simplified Chinese only. Its button and folder names below are given in Chinese with an English translation; iOS, macOS and Windows controls use their English names.

## Getting started

1. Open LanShare (download it from the repository's **Releases** page, or build it yourself, see [Development](#development)). On first launch macOS may ask whether LanShare can accept incoming network connections. Click **Allow**, or the iPhone will not be able to connect.
2. On the computer, click **添加设备** (Add device) and scan the QR code with the iPhone's Camera app. The iPhone and the computer must be on the same Wi-Fi network.
3. When Safari says "This Connection Is Not Private", tap Show Details › visit this website. To get rid of this warning for good, install the computer's certificate after pairing by following the steps on the page (recommended). Before installing it, check that the certificate fingerprint matches the one shown on the computer.
4. On the iPhone, tap **请求连接** (Request connection). Check that the confirmation code is the same on both screens, then click **允许** (Allow) on the computer.
5. In Safari, tap Share › Add to Home Screen so LanShare opens with one tap next time. If an icon you added earlier shows a gray letter, delete it and add it again.

When you upload from **Photo Library** on the iPhone, iOS converts HEIC photos to JPEG and HEVC videos to H.264 (the resolution stays the same). To send the originals, first save them to Files from the Photos app (Share › Save to Files), then pick **Choose File** in LanShare. The page points this out once, on your first upload.

Closing the window keeps LanShare running in the background: in the menu bar and Dock on macOS, and in the notification area (system tray) on Windows. The menu bar or tray icon's menu has **打开 LanShare** (Open LanShare), **暂停共享** (Pause sharing) and **退出 LanShare** (Quit LanShare).

When the computer cannot be reached (it is asleep, or the iPhone is on another network), the Home Screen app still opens and keeps trying to connect. Texts you send meanwhile wait on the iPhone and arrive once it reconnects, and an unsent draft survives closing the app. Tap a message to copy or delete it; a deleted message disappears on both sides. Deleting a file message keeps the file on the computer unless you choose to move it to the Trash.

While an upload runs, the iPhone's screen stays on and the computer does not go to sleep. An upload that would leave too little free space on the computer is refused, and the iPhone shows the numbers: LanShare always leaves 5% of the disk free, at least 2 GB and at most 20 GB.

Settings › **启动** (Startup) can start LanShare when you log in to the Mac (or to Windows), hidden in the menu bar or tray. It is off by default, and LanShare suggests it once after your first pairing. macOS lists it under System Settings › General › Login Items.

Notifications for phone messages have buttons. **复制** (Copy) copies the whole text, not just the preview, and **打开** (Open) opens a file; programs, scripts, installers, disk images, profiles and certificates are only shown in their folder. Clicking a notification opens that conversation. When one phone sends many messages in a row, only the first two pop up and the rest are summed up. The unread count shows on the Dock icon (next to the menu bar icon while LanShare has no Dock icon; in the tray tooltip on Windows). Settings › **通知** (Notifications) can leave message text out of notifications.

The menu bar / tray menu also works the other way round: **发送剪贴板到…** (Send clipboard to…) sends the text, screenshot or picture you copied on the computer to a phone (the most recent conversation, or a device you pick), and **复制最近收到的文字** (Copy latest received text) puts the newest text from a phone on the clipboard. Settings › **剪贴板** (Clipboard) can add a global shortcut for sending, with at least two modifier keys, one of them Control or Command (such as ⌃⌥⌘V); none is set by default. LanShare reads the clipboard only when you do one of these.

LanShare never checks for updates itself. Settings › **查看新版本** (Check for new version) opens the **Releases** page in your browser, where each release lists what changed, both installers and their checksums.

## Windows

Download the installer, `LanShare_<version>_x64-setup.exe`, from the repository's **Releases** page. GitHub Actions ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) also runs every test on Windows and keeps the installer of every build as an artifact: **Actions** tab › a successful run › **Artifacts** › `LanShare-windows-x64` (unzip it to get the `.exe`; artifacts are kept for 90 days). Either way you need to be signed in to GitHub with access to this repository. The Windows version has not been tested by hand yet (see PLAN.md, "Windows testing, later").

- Installation requires administrator rights (LanShare installs to Program Files). The installer adds a firewall rule that lets only devices on the same subnet reach LanShare, and only on networks set to Private. Uninstalling removes the rule.
- If Windows has marked the current Wi-Fi as a Public network, the phone cannot connect. LanShare's settings page and the **添加设备** (Add device) window show a warning asking you to switch the network to Private.
- The installer is not code-signed, so SmartScreen blocks it the first time. Click **More info › Run anyway**.
- When the shared folder is on a USB flash drive or a network share, LanShare refuses to delete files there, from the phone and from its own window: those locations have no Recycle Bin, so a delete would be permanent. Delete them in File Explorer instead.

## Development

Requirements: Xcode Command Line Tools, Rust (`brew install rust`) and Node.js 22+.

```bash
npm install
npm run build          # build the web UI into dist/ (the Rust side serves it from there)
npm run tauri dev      # run the desktop app (debug build)
npx tauri build        # package the .app / .dmg into src-tauri/target/release/bundle/
```

To run only the server, without a window (data goes in `.dev-data/`, separate from your real data):

```bash
npm run headless -- --local-port 53299
```

The terminal prints the address for the phone and the login address for the desktop UI. After changing the frontend, run `npm run build` again and reload the page; the debug server reads files straight from `dist/`.

### Tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml   # Rust unit tests
npx tsc                                           # frontend type check
```

The end-to-end API test pairs a fake phone, so run it against a throwaway headless server. The script reads the server's login URL from `.dev-data/headless.log`, and the LAN port is 39210 unless it was taken (for example while LanShare itself is running), so read the port from the log too:

```bash
mkdir -p .dev-data
npm run headless -- --data .dev-data/smoke --shared .dev-data/smoke-share --local-port 53299 --upload-reserve-mb 0 > .dev-data/headless.log 2>&1 &
# wait until the log shows "Press Ctrl+C" (the first build takes a while), then:
PORT=$(grep -o 'listening on port [0-9]*' .dev-data/headless.log | grep -o '[0-9]*$')
PHONE_OVERRIDE=https://127.0.0.1:$PORT scripts/api_smoke.sh .
```

### Releases

Pushing a version tag on `main` makes GitHub Actions build, check and publish a release with both installers and their checksums. The steps are in [docs/releasing.md](docs/releasing.md).

## Project layout

```
src/                web UI (shared by the desktop window and the iPhone)
  app/ chat/ files/ devices/ settings/ pair/ components/ lib/
src-tauri/src/
  lib.rs            desktop shell: window, tray icon, notifications, folder picker
  core/             server: certificates, pairing, auth, messages, files, SSE, database
src-tauri/examples/headless.rs   run the server without a window
src-tauri/windows/hooks.nsh      Windows installer hooks (firewall rule)
scripts/api_smoke.sh             end-to-end API test
scripts/iphone_check.sh          serves the 15-minute iPhone check page (docs/iphone-check.md)
scripts/release_check.sh         checks a release tag against the code (docs/releasing.md)
.github/workflows/ci.yml         CI: Windows tests and installer
.github/workflows/release.yml    release: a tag vX.Y.Z publishes both installers
```

## Where data lives

- App data (database, certificates, config): `~/Library/Application Support/com.lanshare.desktop/` on macOS, `%APPDATA%\com.lanshare.desktop\` on Windows.
- Shared folder: `~/LanShare` by default (`%USERPROFILE%\LanShare` on Windows). Files received in chat are saved to `收件箱` (Inbox) and files sent to the phone to `已发送` (Sent); these two folders are only visible on the computer.
- Logs: the `logs` folder inside the app data folder (Settings › **打开日志文件夹** (Open log folder)), at most about 10 MB. They never record message text or file names; **详细日志** (Verbose logging) in Settings adds detail for troubleshooting until LanShare restarts.
- Chat history can be deleted automatically after 30 or 90 days (Settings › **自动删除旧聊天记录** (Delete old chat history automatically)). Deleted messages are wiped from the database file, not just hidden.
- LAN port: 39210 by default (if it is taken, LanShare picks another one and remembers it).

## Known limitations

- The connection drops when LanShare goes to the background or the iPhone locks; missed messages are fetched automatically when you return to LanShare. There are no push notifications on a LAN-only setup.
- Locking the iPhone during an upload: in testing, a 600 MB video still finished uploading with the screen locked, but iOS may stop an upload if the phone stays locked for a long time, and interrupted uploads cannot be resumed yet.
- Company, hotel and guest Wi-Fi networks often have "AP isolation" turned on, which stops devices from reaching each other.
- If the computer's IP address changes, old Home Screen icons stop working. Scan the QR code again.
- The app is not signed. On another Mac, allow the first launch in System Settings › Privacy & Security by clicking **Open Anyway**.

## License

LanShare is released under the [MIT License](LICENSE).
