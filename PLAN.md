# LanShare project plan

> A messaging and file-transfer tool for iPhone ↔ computer (macOS / Windows) on the local network. LAN only; nothing goes through any cloud.
>
> Document date: 2026-10-20 · Status: Mac MVP complete (M0–M5, branch `mac-mvp`; for M5 acceptance testing on a real device, see 4.1.1); M6 (Windows) merged into `main` (PR #1); CI still builds and tests every Windows change, and hands-on Windows testing waits until a Windows PC is available; M6.1 done (PR #4); 15-minute iPhone check passed 2026-10-20; next: M7 → M9, planned 2026-10-20 (see section 12) · "LanShare" is a working name and can be changed

## 1. Goals and scope

**In scope:**

- A desktop app (macOS + Windows) that acts as the "server" on the LAN
- The iPhone reaches the desktop app through Safari (pairing by scanning a QR code); no app to install
- Two separate modules:
  - **Messages**: iMessage-style chat; every message has a quick copy button
  - **Files**: the iPhone browses the shared folder on the computer; files transfer both ways, iPhone ↔ computer
- Security and privacy are hard requirements

**Out of scope for now:**

- Internet / cross-network transfer, cloud sync, user accounts
- A native iOS app (later, once there is a paid developer account; see section 11)
- Android (the browser approach works in theory, but there is no Android-specific adaptation or testing)

## 2. Usage flow

**First use**

1. Install and open LanShare on the computer → it generates the certificates and creates the shared folder automatically
2. On the computer, click **添加设备** (Add device); the screen shows a QR code
3. Scan it with the iPhone camera → it opens in Safari → (recommended) install the certificate by following the guide → tap **请求连接** (Request connection)
4. The computer shows a dialog; check that the confirmation codes on both sides match, then click **允许** (Allow) → pairing is done
5. On the iPhone, use "Add to Home Screen" in Safari; after that, one tap opens it

**Daily use**

- Open it from the Home Screen icon; two tabs at the bottom: **消息** (Messages) / **文件** (Files)
- The desktop app keeps running in the background and shows a system notification when a message arrives

## 3. Decisions made

| Topic | Decision | Reason |
|---|---|---|
| Architecture | The computer is the server (Hub); the iPhone is a browser client | No app to install on the phone; data lives only on the computer |
| Desktop tech stack | Tauri 2: Rust core + web UI (React + TypeScript) | The desktop window and the phone web page share one UI codebase; small installer (on the order of 10 MB) |
| Real-time channel | SSE (server push) + plain HTTPS requests | Tested and works; more reliable than WebSocket: WebSocket on older iOS has known issues with self-signed certificates |
| Phone client | iPhone only, as a Safari web page | No paid developer account yet; apps installed with a free account expire after 7 days |
| Windows version | GitHub Actions builds and tests the packages automatically; hands-on testing waits for a Windows PC (changed 2026-10-20) | No Windows computer at hand |
| Native app | Long term; an Australian developer account is an option | Feasible; see section 11 |
| Development order (changed 2026-10-15) | Start with local Git and build the Mac version first; GitHub Actions and the Windows version come after you create the GitHub repository | The GitHub repository will be created later |
| Roadmap after M6 (2026-10-20) | M6.1 → M7 → M8 → M9 (section 12); a milestone that needs real devices is accepted on them in one sitting, and hands-on Windows checks wait for the "Windows testing, later" list | Fix gaps in what LanShare already promises before adding features; the new auth path in M9 comes after the multi-device tests in M6.1 |
| Windows testing (changed 2026-10-20) | No VM. Keep building Windows features and let CI build and test them; once a Windows PC is available, run the "Windows testing, later" list in section 12 in one pass | One combined check on real hardware instead of a VM |
| Pause across restarts (2026-10-20) | If sharing was paused, it stays paused after LanShare restarts, and the phone and Settings show it clearly | Once launch at login exists, a forgotten pause should not quietly turn itself off |
| Automatic chat cleanup (2026-10-20) | Build it in M6.1: off / 30 days / 90 days | Section 7.2 already promises it |
| Unsent text on the phone (2026-10-20) | Drafts and queued (not yet delivered) messages may be kept in Safari's local storage on the phone; they are deleted once sent and on unpairing | iOS often reloads Home Screen apps after you switch away; this is only your own unsent text, on your own phone |
| Deleting a file message (2026-10-20) | The attachment in 收件箱 (Inbox) / 已发送 (Sent) is kept by default; an option also moves it to the Trash / Recycle Bin | Deleting a message should not silently remove a file that may still matter |
| Free space during uploads (2026-10-20) | Uploads always leave max(2 GB, 5% of the disk) free, capped at 20 GB | Protects the computer's disk; the refusal shows the real numbers |
| Launch at login (2026-10-20) | Off by default; suggested once after the first successful pairing | macOS shows a "background item added" notice, so it should be your choice |

## 4. Feasibility results (2026-10-15)

### 4.1 iPhone Safari tests (iOS 27 Simulator, Safari 27)

Ran a test server with a self-signed certificate and tested each item in the Simulator:

| Check | Result |
|---|---|
| First visit | Shows "This Connection Is Not Private"; tap Show Details → visit this website → Visit Website; 3 steps in total, needed only once per address |
| Secure page after clicking through? | ✅ `isSecureContext = true`; all web features that require a secure context work |
| Copy button (Clipboard API) | ✅ The content reached the iPhone clipboard (verified by reading the Simulator clipboard back) |
| Copy button (fallback: execCommand) | ✅ Also works |
| Real-time push (SSE) | ✅ |
| WebSocket (wss) | ✅ Works on iOS 27; older iOS has known issues, so SSE remains the choice |
| System share sheet (`navigator.share`) | ✅ Works; used for **存储到相册** (Save to Photos) |
| After trusting the computer's root certificate on the iPhone | ✅ No warnings at all; the address bar looks normal. iOS accepts the combination "CA with name constraints + 397-day server certificate" |

**Conclusion: the browser approach works on the iPhone.** The Simulator and a real device use the same Safari engine version, but the first step of development (M0) is still a 5-minute re-test on a real device.

### 4.1.1 Real-device results (M5, 2026-10-15, iPhone 16 Pro Max · iOS 27)

| Check | Result |
|---|---|
| Pairing by QR scan | ✅ The QR code holds the LAN address (192.168.x.x), not 127.0.0.1; on first run, the user must click "Allow" in the macOS firewall prompt themselves |
| Upload from "Photo Library" | ⚠️ Always transcoded: HEIC → JPEG, HEVC video → H.264, PNG screenshots → JPEG; resolution stays the same, videos may get larger. Tried 4 `<input accept>` variants (omitted, `image/heic,…`, adding `*/*`, listing extensions); none of them avoids it |
| ProRAW (DNG) | ✅ Uploaded unchanged |
| Via the Files app (in Photos, first Share › Save to Files, then "Choose Files") | ✅ The original uploads unchanged; no conversion step when picking |
| Handling | Do not change iPhone settings ("Settings › Photos › Transfer to Mac or PC › Keep Originals" affects iCloud Photos users and is risky). Instead, show a one-time note on the iPhone the first time the user taps upload (`src/components/PhotoTip.tsx`); the user can pick a quick upload or send originals via Files |
| Screen locked during upload | ✅ Locked the screen right after a ~600 MB video upload started (LAN at 30–40 MB/s); after unlocking, the upload had finished. A longer lock can still get interrupted by iOS (M7 keeps the screen on during uploads; resumable uploads are on the backlog, section 12) |
| Add to Home Screen | ✅ The Home Screen app reuses Safari's pairing credential and opens straight into the chat |
| Without the certificate installed | The Home Screen app shows a certificate warning on every launch; tapping "Visit" gets through |
| Home Screen icon | ✅ (after fix) At first it was a gray letter L, while the icon in the share sheet looked fine. Cause: "Add to Home Screen" downloads apple-touch-icon in a separate system process (SafariViewService), which does not reuse Safari's "continue to visit" choice. In the Simulator it reproduces while the certificate is untrusted and works once it is trusted; on a real device it still failed with the certificate trusted — the computer's log shows some of the phone's background connections rejecting the certificate with `CertificateUnknown`, while Safari and the Home Screen app themselves show no warning. Fix: at build time, embed the icon in the page as a data: URL (`vite.config.ts`), so no download is needed; re-tested on a real device and it works. The icons were also changed to standard sizes (180 / 192 / 512) |
| Certificate profile install | ✅ Fingerprint location: Settings › Profile Downloaded › More Details › certificate name › "Fingerprints" at the very bottom; the SHA-256 matches the one on the computer. During install, iOS shows "will change settings on your iPhone", "Not Verified" and "Unmanaged Root Certificate" — generic warnings shown for every profile; this profile contains only one certificate. Once it is installed and enabled in "Certificate Trust Settings", no warnings remain |
| Certificate name | ✅ (after fix) Every root certificate used to be named "LanShare Local CA (Mac)": each dev run, the release build and every **重置证书** (Reset certificate) created its own, and they looked identical in "VPN & Device Management" and "Certificate Trust Settings", so there was no way to tell which one was old. New certificate names now end with a 6-character ID (taken from the certificate's key identifier, e.g. "LanShare Local CA (Mac) 2DD31B"), and both **设置 › 安全** (Settings › Security) in the desktop app and the iPhone install guide show the current certificate's full name. Old certificates that are already installed are not regenerated just to rename them; their names stay the same |
| The `.local` name in the certificate | ✅ (after fix) It used to come from the hostname, which on this Mac is `mac.local` (a hostname assigned by the router), while Bonjour actually answers to `MyMac.local`, so `.local` access (now planned for M9) would fail. On macOS it now uses the Bonjour local hostname (System Settings › General › Sharing › Local hostname) and falls back to the hostname only when that is unavailable; after a rename, the server certificate is re-signed automatically |
| Computer name | ✅ (after fix) The computer name in the phone's top bar, on the pairing page and in new certificate names used to come from the hostname, which shows as "Mac" on this Mac. On macOS it now uses "Computer name" (System Settings › General › Sharing, i.e. `scutil --get ComputerName`, here "MyMac") and falls back to the hostname only when that is unavailable; Windows is unchanged. A rename takes effect after restarting LanShare; existing root certificates are not regenerated because of it |

### 4.1.2 15-minute iPhone check (2026-10-20, real iPhone · iOS 27.2)

Run with the page and steps in `docs/iphone-check.md`, against the desktop app and the CA the iPhone already trusts:

| Check | Result |
|---|---|
| Safari opens `https://mymac.local:39210` with the certificate trusted | ✅ No warning. Two things had to be put right first: the iPhone no longer trusted the CA (turned back on in Certificate Trust Settings), and the installed build from 2026-10-15 predated the Bonjour-name fix, so its certificate named `mac.local` (rebuilt from `main`, same CA) |
| Shortcut "Get Contents of URL" on `/api/ca` over `.local` | ✅ Returns the CA name. iOS asks once per address whether the action may connect; before the CA was trusted it said the certificate was invalid |
| The same Shortcut over the IP | ✅ Same result |
| `<input type="file" webkitdirectory>` picks a whole folder | ✅ Works (the Simulator also returned every file with its folder path) |
| The share sheet behind **存储到相册** (Save to Photos) | ✅ Copy and Save Image among the actions, WeChat among the apps, plus Save to Files and AirDrop |

Consequences for section 12: M9 builds both the `.local` address and the Shortcuts; folder upload goes on the backlog; no separate "copy image" button. Also seen: the pairing cookie belongs to one address, so a phone paired over the IP sees the pairing page at `mymac.local`, which M9's `.local` QR code and **替换这台旧设备** (Replace this old device) have to cover.

### 4.2 Windows version (no Windows machine at hand)

- **Packaging**: the official `tauri-apps/tauri-action@v1` builds on a GitHub-hosted Windows machine (`windows-latest`) and produces `.msi` / `.exe` installers. They can simply be downloaded as build artifacts; there is no need to publish a Release.
- **Cost**: private repositories get 2,000 free minutes per month; Windows machines cost more per minute than Linux, so the quota runs out faster. A cold Rust build takes about 10–20 minutes, which is enough for a personal project. **With no payment method attached, nothing is charged**; when the quota runs out, runs just pause. Public repositories are completely free.
- **Testing**:
  1. Automated tests: every push to `main` and every PR runs the Rust unit tests and API tests on Windows (path rules, file reads and writes, API) and builds the installer; from M6.1, CI also installs and uninstalls it silently and checks the firewall rule
  2. Manual tests (changed 2026-10-20: no VM): once a Windows PC is available, install the package CI builds and use it for real, following the "Windows testing, later" list in section 12. If that PC runs Windows on ARM, the x64 installer runs under emulation (an ARM64 build is on the backlog)
- **Code signing**: without a signature, Windows shows "Windows protected your PC" on first run; click "More info → Run anyway". Fine for personal use; buy code signing later if it becomes necessary.

### 4.3 Native app and an Australian developer account

Conclusion: **feasible**; see section 11 for details.

## 5. System architecture

```
LanShare desktop app (macOS / Windows, Tauri 2)
├─ Window: web UI, loads http://127.0.0.1:<random port> (local service, not exposed)
├─ Rust core
│  ├─ LAN service: HTTPS, 0.0.0.0:39210 (next port if taken), accepts private IPs only
│  ├─ Local service: HTTP, listens on 127.0.0.1 only, used only by the desktop app's own window, logs in with a random token generated at startup
│  ├─ Certificates: local root certificate (CA) + server certificate, generated automatically, renewed automatically when the IP changes
│  ├─ Pairing and auth, messages, files, real-time push (SSE)
│  └─ Storage: SQLite (devices, messages) + shared folder (the files themselves)
└─ System integration: tray / menu bar, system notifications, Show in Finder / File Explorer, launch at login
          ▲
          │  Same Wi-Fi, HTTPS-encrypted
          ▼
iPhone Safari: opens https://<computer IP>:39210; after pairing, it authenticates with the credential stored in a cookie
```

Key points:

- Both services share the same API and web UI. Only "local service + local token" has host privileges (approving pairing, managing devices, changing settings); requests from the LAN can never get them.
- The API is not tied to a specific client: to add a native app later, the API only needs to also accept `Authorization: Bearer` tokens.

## 6. Functional spec

### 6.1 Messages module

- One conversation per paired device. The desktop app has a conversation list; the iPhone sees only its own conversation with the computer
- Message types: text, image / file
- Bubble layout: your own messages on the right (blue), the other side's on the left; grouped by time; send status shown (sending / delivered / failed, with retry)
- **Copy button**: every text message has a small, always-visible button next to it. One tap copies the text, the icon turns into ✓, and a **已复制** (Copied) toast appears; long-press to select only part of the text
- Links are clickable (only http / https are recognized); long text and code are shown in a monospace font
- History loads in pages; after a disconnect the client reconnects automatically and fills in the messages it missed; failed sends are queued and resent automatically (deduplicated by client_id)
- Desktop app: drag a file into the chat window to send it; paste a screenshot to send it directly; incoming messages show a system notification
- Image messages show a preview. File messages show the file name and size: on the iPhone, tapping one downloads / previews it; on the computer, clicking one opens it in the default app or shows it in its folder
- Where files sent in chat are stored: iPhone → computer in `<shared folder>/收件箱/` (Inbox); computer → iPhone in `<shared folder>/已发送/` (Sent). These two folders are visible only on the computer. Each phone can open attachments only through its own chat messages (so devices cannot see each other's files)

### 6.2 Files module

- The shared folder defaults to `~/LanShare` (on Windows, `C:\Users\<you>\LanShare`) and can be changed in settings
- Browsing: the path is shown at the top, folders come first, and each entry shows name, size and modified time; files starting with `.` and system files are hidden
- Preview: images, video (with a draggable progress bar, via HTTP Range), audio, PDF, plain text
- iPhone download: saves to the Files app; images and videos can be saved to Photos from the share sheet ("Save Image" / "Save Video")
- iPhone upload: pick from the photo library, the camera or Files; supports multi-select; shows progress and speed; can be cancelled; on a name clash the file is renamed to `name (1).jpg` automatically
- Management: new folder, rename, delete (**delete moves items to the Trash / Recycle Bin; it is not permanent**)
- Permissions: each device can be set individually to **读写** (Read-write) or **只读** (Read-only)
- Live refresh: when any side changes a file, other devices viewing the same folder refresh automatically
- Later (backlog, section 12): thumbnails, multi-select and download as a ZIP, resumable uploads

### 6.3 Devices and pairing

1. On the desktop app, click **添加设备** (Add device). It shows a QR code (the link carries a one-time token, valid for 2 minutes) and a 6-digit pairing code (for manual entry when the QR code cannot be scanned)
2. Scan the code with the iPhone Camera and open it in Safari (the first time, tap through the certificate warning, or install the certificate first) → enter a device name (default "iPhone") → **请求连接** (Request connection)
3. Both sides show the 4-digit **confirmation code** at the same time, and the computer shows a dialog: "iPhone 请求连接，确认码 4827，是否允许？" (iPhone wants to connect, confirmation code 4827. Allow?)
4. After **允许** (Allow) is clicked, the iPhone receives a long-lived credential (HttpOnly + Secure cookie). From then on, the iPhone can use the app directly by opening the URL or tapping the Home Screen icon
5. The **设备** (Devices) page on the desktop app shows each device's online status and last activity time, and lets you rename a device, change its permissions or remove it. Removing a device invalidates its credential immediately and deletes that device's chat history

### 6.4 UI

**Desktop window**: a column of icons on the left for Messages / Files / Devices / Settings

- Messages: conversation list + chat area
- Files: shared folder browser, with two actions the phone does not have: **在 Finder / 资源管理器中显示** (Show in Finder / File Explorer) and **用默认程序打开** (Open with default app)
- Devices: device list + Add device (QR code panel)
- Settings: shared folder location, LAN address and port, certificate fingerprint, **暂停共享** (Pause sharing) toggle, reset certificate, clear chat history
- Closing the window keeps the app running in the background: on macOS, click the Dock icon to bring it back; on Windows, restore it from the tray

**iPhone web page**: two tabs at the bottom, Messages / Files

- Handles the notch and the bottom safe area; the input box stays visible when the keyboard opens; the input font size is at least 16px so Safari does not zoom the page
- The part of the URL after `#` records the current page and folder path, so swiping back in Safari goes up one level as expected
- Supports "Add to Home Screen" (with its own icon; opens full screen)
- Certificate install guide page (illustrated steps)

## 7. Security and privacy design

### 7.1 What to defend against

Other devices on the same Wi-Fi (roommates, guests, strangers on a public network, compromised smart home devices) may:

- Scan ports, impersonate your computer (man-in-the-middle), or eavesdrop on traffic
- Brute-force the pairing code
- Use the file API to read files outside the shared folder
- Send malicious content to attack the web page

### 7.2 Measures

| Risk | Measure |
|---|---|
| Eavesdropping | HTTPS everywhere (TLS 1.2 / 1.3); no unencrypted endpoint is offered on the LAN |
| Someone impersonates the computer (man-in-the-middle) | We recommend installing the root certificate generated by the computer on the iPhone and **checking the certificate fingerprint** before installing (the SHA-256 shown on the computer screen must match the one in the profile details on the iPhone). Once a checked certificate is installed, Safari validates strictly and an impostor cannot get through. If you install without checking, the first certificate download can still be spoofed; without the certificate, protection relies only on the network itself being trustworthy |
| Unauthorized access | Pairing requires manual approval on the computer and a confirmation code check; after that, a 256-bit random credential is used (the computer stores only its hash), and it can be revoked at any time |
| Brute-forcing the code | The QR code token is single-use and expires after 2 minutes; the 6-digit pairing code is rate-limited (5 attempts per IP per minute; the code is invalidated after 10 failures in total) |
| Reading files outside the shared folder | All paths are confined to the shared folder: `..`, absolute paths, symlinks that point outside, hidden files, Windows reserved names (CON, NUL, etc.) and other special forms are rejected |
| Cross-site request forgery | Cookies are set to SameSite=Strict; every write must carry a custom request header, and Origin is checked |
| Web page injection (XSS) | Messages are always shown as plain text; a strict Content Security Policy (CSP) is enabled; HTML, SVG and similar files in the shared folder can only be downloaded or opened in a sandbox, and cannot run scripts in the app's pages |
| A malicious web page reaches the local service indirectly (DNS rebinding) | The local service listens only on 127.0.0.1, checks the Host header, and requires the random token generated at startup |
| Access from outside the LAN | Accept only private IPs (10.x, 172.16–31.x, 192.168.x, link-local addresses); no UPnP port mapping; the Windows firewall rule allows only "Private network" |
| A received file is run as a program | Files from the phone get the "from the internet" quarantine flag on macOS (Zone.Identifier on Windows); when the desktop app opens a program or script file, it shows the file in its folder instead of running it; invisible characters in file names that can disguise the extension are removed |
| Sharing too much | The user's home folder itself, its parent folders, the system Library folders and LanShare's own data directory cannot be set as the shared folder; access to the data directory is denied even when it falls inside the shared scope |
| Idle connections exhaust the server | Drop non-LAN connections before the TLS handshake; at most 32 concurrent connections per IP and 256 in total; a read timeout for request headers; raise the open-file limit |
| Certificate private key leak | The root certificate's private key exists only on the local computer, is readable only by the current user, and is never sent out. The root certificate has "name constraints", so it can only issue certificates for LAN addresses; even if leaked, it cannot impersonate outside websites. Settings has a one-click reset |
| Privacy | No accounts, no servers, no data reporting; logs never record message text or file names (from M6.1); chat history is stored only on this computer (only unsent drafts and queued messages may wait on the phone; see section 3) and can be cleared, or set to clean up automatically on a schedule |
| Temporarily blocking access | One-click **暂停共享** (Pause sharing) immediately rejects all requests from the LAN |

### 7.3 Certificate scheme

**Generated at first launch:**

- Local root certificate (CA): ECDSA P-256, valid for 10 years. It has name constraints that allow only `localhost`, `*.local` and private IP ranges
  - The name looks like "LanShare Local CA (computer name) 2DD31B" (on macOS the computer name is the "Computer Name" setting; on other systems it is the hostname). The last 6 characters are the start of this certificate's key identifier. They differ on every generation (resets included), so you can tell new from old on the iPhone. Certificates generated by older versions keep their original name and are not regenerated
- Server certificate: issued by the root certificate above
  - The addresses in the certificate include `localhost`, `<computer name>.local` and all of this computer's private IPs. On macOS the `.local` name comes from the Bonjour local hostname (`scutil --get LocalHostName`); on other systems, or when that is unavailable, the hostname is used
  - Usage limited to server authentication (serverAuth)
  - Valid for 397 days, which meets Apple's requirements for trusted certificates
  - Reissued automatically 30 days before expiry, when the IP changes, or when the computer is renamed, and takes effect immediately (no restart needed)

**The iPhone has two options:**

1. Quick: tap through the warning on the first visit (3 steps)
2. Recommended: after pairing, tap **安装证书** (Install certificate) to download the profile; before installing, check in the profile's "More Details" that the SHA-256 matches the one shown on the computer, then install it and turn on full trust in General › About › Certificate Trust Settings. After that, there are no more warnings, and man-in-the-middle attacks are actually blocked

## 8. API design (draft)

All endpoints start with `/api` and use JSON; write requests must carry the `X-LanShare: 1` request header.

| Method | Path | Description | Access |
|---|---|---|---|
| POST | /pair/request | Start pairing with the token or the 6-digit code; returns request_id and the confirmation code | Public (rate-limited) |
| GET | /pair/status/{id} | Poll the pairing result; sets the cookie once approved | Public |
| GET | /ca | Name of the current root certificate (shown in the install guide, so the user knows which switch to turn on) | Public |
| GET | /ca.crt | Download the root certificate (public information) | Public |
| GET | /me | Current identity | Paired |
| GET | /events | SSE real-time events | Paired |
| GET / POST | /messages | Fetch history (paged by before / after) / send text | Paired |
| POST | /messages/upload | Send a file message (the request body is the file stream) | Paired |
| GET | /messages/{id}/file | Download or preview a message attachment (supports Range) | Paired |
| GET | /files?path= | List a directory | Paired |
| GET | /files/download?path=&inline= | Download or preview a file (supports Range) | Paired |
| POST | /files/upload?dir=&name= | Upload (written to a temp file as it arrives, renamed to the final name when complete) | Read-write |
| POST | /files/mkdir, /files/rename, /files/delete | New folder / rename / delete to the Trash / Recycle Bin | Read-write |
| * | /host/... | Pairing QR code, approval, device management, settings, opening files / folders / links | Host only |

SSE events:

| Event | Meaning |
|---|---|
| `msg` | New message |
| `files` | A directory changed |
| `devices` | Device list or online status changed |
| `pairing` | New pairing request (sent to the host only) |
| `revoked` | The current device was removed |
| `paused` | The computer paused sharing |
| `resync` | Some pushes were missed; the client must fetch the data again |

## 9. Data and storage

- The app data directory (`~/Library/Application Support/<id>/` on macOS, `%APPDATA%\<id>\` on Windows) contains:
  - `lanshare.db` (SQLite database)
  - `config.json` (shared folder path, port, pause state, chat cleanup period)
  - `tls/` (certificates and private keys)
  - `logs/` (the log file, rotated, at most about 10 MB; from M6.1)
- Tables:
  - `devices`: id, name, credential hash, read/write permission, created time, last activity time
  - `messages`: id, conversation (device), direction (sent by the computer / sent by the device), client_id (for deduplication), type, text, attachment relative path / file name / size / type, time
- Files themselves live only in the shared folder; a message records only the file's relative path

## 10. Project structure (planned)

```
NetworkFileShare/
├─ PLAN.md, README.md
├─ package.json, vite.config.ts, index.html
├─ src/                    # web UI (shared by the desktop window and the iPhone)
│  ├─ app/                 # startup, routing, host / device layouts
│  ├─ chat/                # messages module
│  ├─ files/               # files module
│  ├─ devices/, settings/, pair/
│  └─ lib/                 # API wrappers, SSE, copy, formatting
├─ src-tauri/
│  ├─ src/
│  │  ├─ lib.rs            # Tauri window, tray, notifications
│  │  ├─ server/           # routing, auth, pairing, messages, files, SSE
│  │  └─ tls.rs, db.rs, paths.rs, net.rs
│  ├─ examples/headless.rs # runs the server alone, without a window (for testing)
│  └─ tests/               # API integration tests
└─ .github/workflows/      # CI: tests + Windows / macOS packaging
```

## 11. Native iOS app and an Australian developer account (long term)

| Option | Verdict |
|---|---|
| Free Apple account | Can install apps, but not fit for daily use: installed apps **stop working after 7 days**; at most 3 registered devices, at most 3 apps per device; you must reinstall with Xcode regularly. Good for trying out a prototype first |
| Paid developer account (Australia) | **Feasible**. The annual fee is US$99, charged in local currency (some users report the Australian price as A$149; the price shown on the enrollment page is what counts) |

**Enrollment requirements (from Apple):**

- The Apple Account must have two-factor authentication turned on, and the name, address, phone number and other details must be real and valid
- Enrolling as an individual requires your **real legal name**. This name is shown as the seller name on the App Store
- Enrolling through the Apple Developer app requires taking a photo of your ID (most regions accept a passport, some also accept a driver's license), and the whole enrollment must be done on the same device
- You cannot pay with an Apple Gift Card balance. You must use a payment method that works locally
- The account's region follows the enrollee's real identity and address. If you use a relative's or friend's account, the account holder should enroll themselves and then sign the app for your iPhone

**What a paid account gives you:**

- Signing is valid for one year. You can install directly to your own iPhone with Xcode, or go through TestFlight
- Installing directly with Xcode is authorized per device, so the region of your iPhone's Apple ID does not affect installation
- Side benefit: the macOS version can also get developer signing and notarization, so the system does not block it when other people download it
- If the app is only for personal use and not published on the App Store, the ICP filing required to publish in the China region does not apply

**How to do it later:** The server API does not change. For the client, pick one of two options:

1. Use Tauri 2 to package the same web UI directly as an iOS app (smallest change)
2. Write a native app in SwiftUI (best experience: one tap in the system share sheet sends to the computer, background transfers, finding the computer automatically through Bonjour)

A native app has limits too: the iPhone cannot receive real-time messages while in the background, because there is no push over the internet. iOS has a Local Push Connectivity feature that can do local push on specified Wi-Fi networks, but it is complex to configure. Evaluate it when the time comes.

## 12. Development milestones

Each milestone has a result that can be verified in acceptance testing. Finish one before starting the next. The exception is hands-on Windows testing: those checks are collected in the "Windows testing, later" list below and done in one pass on a Windows PC, so M6 stays open while the later milestones go ahead.

**M0 Environment and real-device check**

- Install Rust, generate the project skeleton
- Open a test page on a real iPhone (about 5 minutes): certificate warning, copy, real-time push, format of uploaded photos
- Initialize Git in the local project folder (link the GitHub repository once you have created it; see M6)
- Acceptance: every item on the test page passes on a real device

**M1 Server core**

- Certificate generation and hot reload, listeners for both servers, private IP filtering
- Pairing (QR code + 6-digit code + confirmation code), cookie auth, SSE, device management API
- Rust unit tests: path restrictions, filename sanitization, credential checks, every state of the pairing flow, rate limiting
- Acceptance: complete the whole pairing flow from the command line; every request from an unpaired device returns 401; requests with a wrong Origin are rejected

**M2 Messages module**

- Messages API, and the chat UI in two layouts: desktop and iPhone
- Copy button, resend on failure, catching up on missed messages after a disconnect, file messages
- Acceptance: send text, images and files both ways between the desktop window and the iPhone Simulator; the copy button really writes its content to the clipboard; no messages are lost after the network drops and reconnects

**M3 Files module**

- Browse, preview, download (with Range), upload (written to disk while receiving + progress)
- New folder, rename, delete to the Trash / Recycle Bin, read-write / read-only permissions, live refresh
- Acceptance: after the iPhone uploads a 1GB video, the computer opens it normally; the video plays with a draggable progress bar; every path-escape variant is rejected

**M4 macOS desktop app**

- Tauri window, Devices page, QR code, pairing dialog, Settings page
- System notifications, keep running in the background after the window closes, menu bar icon, package .app / .dmg
- Acceptance: install and run on your Mac, pair a real iPhone by scanning the QR code, and send and receive messages and files

**M5 iPhone experience polish**

- Certificate install guide, Add to Home Screen, keyboard and safe area handling
- Saving to Photos, handling HEIC and video uploads
- Acceptance: go through the whole flow on a real device: "first scan → install certificate → daily use"
- Status: ✅ Done (see 4.1.1 for results). Also fixed: if the shared folder is deleted, it is recreated in its original location (it used to fall back to `~/LanShare`); relative paths in the `headless` example resolve against the current directory. The desktop app's debug log records the phone's page requests (excluding `/api/`) and TLS handshake failures, to help troubleshoot connection problems

**M6 Windows version**

- Prerequisite: start only after you create the GitHub repository and give me its URL (finish the Mac version before that)
- Windows adaptation: path rules, Recycle Bin, firewall prompt, tray, allow only one running instance
- Status: merged into `main` through PR #1 (2026-10-15). CI on Windows passes clippy, all Rust tests and the API end-to-end test, and builds the installer. **Not accepted yet: the installer has not been run on Windows; that happens with the other Windows checks once a Windows PC is available.**
- Done (2026-10-15; the code is on GitHub at DEFY-AN94/LanShare):
  - The installer (NSIS, installs to Program Files) adds a firewall rule automatically: it allows only Private networks and the local subnet; uninstalling removes it
  - At runtime, detect when Windows is set to "Public network", and prompt on the Settings page and in the Add device window to switch it to a Private network
  - Before deleting, check the drive type, and refuse to delete on USB flash drives / network shares (they have no Recycle Bin); run Recycle Bin operations on a separate thread to avoid COM initialization conflicts
  - VPN adapters (point-to-point links, common VPN names) no longer appear in the QR code address
  - Upload handlers read any request body they left unread before answering; otherwise the connection reset on Windows discarded the response, and a retried upload that had gone through looked failed
  - All `cfg(windows)` code (hidden attribute, Zone.Identifier, long paths, UNC paths) compiles in CI, and all Rust tests pass on Windows
  - CI: on Windows, run clippy, all Rust tests and the API end-to-end test, and build the Windows installer; the end-to-end test fails the job when any check fails (macOS tests run only on PRs)
- GitHub Actions: on every push to `main` and on every PR, run the tests and package the NSIS `.exe` installer as the build artifact `LanShare-windows-x64`
- Still to test on a real Windows PC: see "Windows testing, later" in section 12
- Acceptance: Actions produces the installer (done); installing it on Windows and completing pairing and transfers with an iPhone are in "Windows testing, later"

**15-minute iPhone check** (no code; ✅ done 2026-10-20, all four passed, see 4.1.2)

A test page and a one-action Shortcut, run on the real iPhone with the LanShare certificate trusted. Each answer decides part of the plan:

1. Safari opens `https://mymac.local:39210` with no warning → the `.local` part of M9
2. A Shortcut with a single "Get Contents of URL" action reaches `/api/ca` over `.local` and over the IP and passes the certificate check → whether the Shortcuts part of M9 is built
3. `<input type="file" webkitdirectory>` lets you pick a whole folder (WebKit fixed this for iOS 18.4) → whether folder upload goes on the backlog
4. The system share sheet that **存储到相册** (Save to Photos) opens already offers Copy and WeChat → whether a separate "copy image" button is needed

**M6.1 Fix gaps in what LanShare already promises** (about 1 week)

- Status: ✅ done 2026-10-20 (PR #4); its Windows parts are checked by CI, and by hand later ("Windows testing, later")
- Log file for installed builds. Today release builds keep no logs: tracing goes to stdout only (`lib.rs`), and Windows release builds have no console. Write a size-capped log to `<data dir>/logs`, switched on once the data directory is known. Settings gets **打开日志文件夹** (Open log folder) and a **详细日志** (Verbose logging) switch that resets on restart. Logs never contain message text or file names: scrub the current lines that log paths (`error.rs`, `host.rs`, `mod.rs`) and the headless bridge's notification line, which logs message text and file names (`bridge.rs`; the API tests and the smoke test use that bridge), and a test checks that marker strings never reach the log
- Deleted chat text is really gone. SQLite's `secure_delete` is off (`db.rs` sets only WAL, foreign keys and the busy timeout), so cleared text stays readable in `lanshare.db` and its WAL file. Turn it on, and run one purge step (`PRAGMA wal_checkpoint(TRUNCATE)`; a plain checkpoint leaves old frames in the WAL file) after every delete path: clearing chats, removing a device, unpairing from the phone, and the single-message delete in M7
- Automatic chat cleanup (section 3): a Settings choice of off / 30 days / 90 days, run at startup, every 6 hours and whenever the setting changes, through the same purge step
- Close the chat-folder isolation gaps. The 收件箱 / 已发送 check looks at the requested path, while file lookups follow symbolic links, so a link that the computer's user creates to 收件箱 gets past it (`routes/files.rs`). Check the resolved path instead, and drop listed entries that resolve into a chat folder. `files` events about 收件箱 / 已发送 go to the host only (today every device gets them, `state.rs`)
- Pause survives a restart (section 3): store it in `config.json`
- Add Windows file types that run or mount to the reveal-only list for **用默认程序打开** (Open with default app) in `routes/host.rs`: `.chm`, `.msix` / `.appx` / `.appinstaller`, `.iso` / `.img` / `.vhd` / `.vhdx`, with table tests
- Tests: a two-device test that proves devices cannot see each other's messages, attachments or events (every test pairs only one device today); revoking a device mid-upload gives 503 and leaves no `.part` file; fix the smoke test's SVG check, which downloads the HTML file again instead of an SVG
- CI: a job that installs the NSIS build silently, checks the firewall rule (Private profile, local subnet, TCP, the exe in Program Files), starts the app and checks `/api/ca` if the runner allows it, stops the app, uninstalls (waiting for the uninstaller to finish, since it relaunches itself from `%TEMP%`) and checks that the rule is gone. Move GitHub Actions to their Node 24 versions (Node 20 is deprecated)
- Needs from you: no device session (the log file uses a small writer of LanShare's own, so `tracing-appender` is not needed)
- Acceptance:
  - CI installs and uninstalls the NSIS build and checks the firewall rule both times; any failure fails the job; no Node 20 deprecation warnings
  - The installed app on macOS writes `<data dir>/logs`, and **打开日志文件夹** shows it (on Windows, this is checked in "Windows testing, later"); a test proves message text and file names never reach the log
  - After clearing chats, removing a device or unpairing, a marker string is in neither `lanshare.db` nor its WAL file (test); messages older than the chosen cleanup period disappear
  - For a phone, a symbolic link to 收件箱 is not listed and its paths return 403, while the computer still sees it (test); a second phone gets no `files` events for 收件箱 / 已发送
  - Pause, quit and relaunch: LanShare starts paused, phones get 503, and the tray and Settings show it

**M7 Daily iPhone use** (about 2 weeks)

- Messages are not lost. The Home Screen app reconnects by itself when it opens: each attempt times out after about 4 s (today a request to the LAN address can hang for about 60 s when cellular data is on, and the **连接不上电脑** (Can't reach the computer) screen waits for a tap on **重试** (Retry)). You can send while disconnected: messages queue and arrive exactly once. Drafts and queued messages survive page reloads (kept on the phone, see section 3)
- The copy button's tap area grows from 28 px to 44 pt (Apple's minimum) without changing how it looks
- Delete a single message, on the phone and on the computer, through the M6.1 purge step; the attachment follows the section 3 decision
- Uploads: the screen stays on (Wake Lock) while uploads run; the failure reason is shown as text, and 400 / 403 / 409 errors offer no **重试**; after the network comes back, the Files queue retries by itself at most 3 times. Before auto-retry is turned on, Files uploads get an upload ID, so a retry whose first attempt already landed does not create a `(1)` copy
- The computer does not idle-sleep while a transfer is active (macOS: a power assertion; Windows: `SetThreadExecutionState`), and **电脑可能已睡眠** (The computer may be asleep) is added to the connection-error causes
- Upload preflight: refuse uploads that would break the free-space reserve (section 3) with 507 and the real numbers, refuse files over 4 GB on FAT32, compare the received size with the declared one, and fsync before the rename. The reservation is taken from the upload request's `Content-Length`, not from a separate call, so it cannot leak
- Files page: sort by name / modified time / size, and filter by name (client-side, remembered on the phone)
- Launch at login on the Mac, hidden in the menu bar (off by default; suggested once after the first pairing). On macOS 13+ the switch reads the real state through SMAppService; first check that this works for an unsigned build, otherwise document the limitation
- First frontend unit tests (vitest, dev-only), run in CI: the chat store's merge, pending and retry logic
- Needs from you: about 30 minutes on the iPhone (Safari and the Home Screen app); approval of vitest
- Acceptance:
  - Open the Home Screen app with Wi-Fi off and cellular on, then turn Wi-Fi on: it connects within about 5 s without tapping **重试**; the same with airplane mode
  - A text sent while the Mac is unreachable shows **等待连接…** (Waiting for connection…) and arrives exactly once; a draft survives switching tabs and force-quitting the Home Screen app
  - The copy button's tap area is at least 44 × 44 pt; deleting a message removes it on both sides live, and its marker text is gone from the database files (test)
  - With Auto-Lock at 30 s, a 2-minute upload keeps the screen on; dropping the connection after the body is sent leaves no `(1)` copy; an upload the free-space reserve does not allow is refused as soon as its headers arrive, before anything is written to disk, and the phone shows why
  - With a 1-minute idle-sleep setting, a 3-minute upload completes
  - With launch at login on, logging out and back in starts LanShare with no window, and the phone connects
  - `npm test` runs in CI and passes

**M8 Quick copy between the computer and the phone, both ways** (about 2 weeks)

- Start with a half-day spike on the Mac: do notification buttons appear for an unsigned app in the default banner style (this may need `NSUserNotificationAlertStyle` in `Info.plist`), does a click from Notification Center still arrive after the banner is gone, and do threads pile up after 50 notifications
- Notifications you can act on: clicking opens that conversation; a **复制** (Copy) button copies the full text, not the 80-character preview; files get **打开** (Open), which only reveals risky file types; the Dock shows the unread count (on Windows, the tray tooltip); at most two notifications per device per burst; an option to hide message text in notifications
- A fallback that always works: **复制最近收到的文字** (Copy latest received text) in the menu bar / tray menu
- Send the computer's clipboard to the phone: a tray item, plus an optional global shortcut (none set by default). It goes to the most recent conversation, with a per-device submenu; a screenshot on the clipboard arrives as a PNG under 已发送. The clipboard is read only when you trigger it
- Launch at login on Windows (Run key) and its cleanup on uninstall (tested later on a Windows PC)
- Needs from you: approval of `tauri-plugin-global-shortcut`, `tauri-plugin-autostart` and a clipboard crate (which also brings in `image`; `png` is already in the tree through Tauri's menu and tray crates); decisions on the shortcut and the target device; about 30 minutes on the Mac
- Acceptance:
  - With the window hidden, clicking a phone message's notification opens that conversation
  - If the spike passes, **复制** on a 300-character message puts all 300 characters on the clipboard without opening the window; either way, the tray item copies it
  - Copy a URL on the Mac and use the tray item or the shortcut: it appears in the phone's chat without the window coming forward; an empty clipboard gives an error notification and sends nothing
  - If the OS refuses a shortcut registration, Settings shows the error and keeps the old shortcut (other apps' in-app shortcuts cannot be detected; Settings says so)

**M9 Fixed address and iOS Shortcuts** (about 2 weeks; the 15-minute iPhone check confirmed both parts)

- Connect by computer name (`mymac.local`). Add device also offers a `.local` QR code, with the IP one as a fallback; whether it becomes the default on macOS is decided when M9 starts, after the real-iPhone test passes. A Home Screen icon added from it survives IP changes. When a phone pairs again, the computer can choose **替换这台旧设备** (Replace this old device), so the device keeps its ID and chat history. The listener stays IPv4-only: Safari then falls back to IPv4 at once when `.local` also resolves to an IPv6 address The pairing cookie belongs to one address (seen in the iPhone check), so a phone paired over the IP is not paired at the `.local` address until it pairs there
- iOS Shortcuts (the 15-minute iPhone check confirmed that a Shortcut reaches LanShare over `.local` and over the IP; iOS asks once per address). A send-only token per device can post messages and files but read nothing. One Shortcut in the share sheet sends text, links, photos and files; an optional second scope, off by default, lets a Shortcut copy the latest text from the computer. The illustrated Chinese recipe keeps the token in a file under On My iPhone (not iCloud Drive) and reads it at run time, so the Shortcut itself holds no secret and does not carry it through iCloud
- Needs from you: about 45 minutes on the iPhone, including an IP change; decisions on making `.local` the default and on the Shortcuts details
- Acceptance:
  - A real iPhone pairs through the `.local` QR code with no warning when the CA is trusted; after the Mac's IP changes, the Home Screen icon reconnects with its chat history
  - Replacing an old device keeps its ID and messages and invalidates the old credential; a phone cannot trigger a replacement (test)
  - A send-only token can post to `/api/messages` and `/api/messages/upload` and gets 401 / 403 everywhere else (tests); revoking it takes effect at once
  - On the real iPhone, a Safari URL, two photos and a PDF from Files shared through the Shortcut all arrive on the Mac

**Windows testing, later** (decided 2026-10-20: no VM; one pass once a Windows PC is available; about 1.5 hours, with the iPhone)

Windows features are still built, and CI builds and tests every change on Windows (from M6.1 it also installs and uninstalls the installer). Everything that needs a person at a Windows PC is collected here, and this pass also closes M6:

- From M6: install and uninstall (SmartScreen and administrator prompts; the firewall rule is added for Private networks and the local subnet only, and removed again), the "Public network" warning on the Settings page and in the Add device window, delete refusal on USB flash drives and network shares (a network drive mapped from a share on the Mac works too), the QR address with a full-tunnel VPN connected (a WireGuard tunnel to a dummy peer needs no VPN service), the app window, tray icon and notifications, and Zone.Identifier on files from the phone
- Pair the real iPhone, send text, a photo and a 1 GB file both ways, and install over the previous build (devices and chat history must be kept). Download the installer on the PC itself, so SmartScreen sees it as downloaded
- **用默认程序打开** (Open with default app) on a phone-sent `.exe` / `.ps1` / `.lnk` / `.chm` shows it in File Explorer instead of running it
- Possible false alarm: the Public-network warning fires when any network profile is Public (`net.rs`), so a VPN or Hyper-V adapter may cause a false warning
- From M6.1: the log file in `<data dir>\logs` and **打开日志文件夹** (Open log folder); a phone cannot reach 收件箱 / 已发送 through a junction or an 8.3 short name (the symlink test runs only on macOS, and no test covers 8.3 names; a Windows CI test would be better if the runner's disk has 8.3 names)
- From M7: with a 1-minute idle-sleep setting, a 3-minute upload completes (`SetThreadExecutionState`); with the shared folder on a FAT32 USB stick, a file over 4 GB is refused
- From M8: clicking a toast, or clicking it later in Action Center, opens that conversation; the toast's **复制** (Copy) and **打开** (Open) buttons; the unread count in the tray tooltip; the tray items that copy the latest text and send the clipboard (a screenshot arrives as a PNG); launch at login (Run key) and its cleanup on uninstall. Record any differences from the Mac
- From M9: the `.local` URL either works or is not offered
- Record the results in section 4.2, in the same format as the 4.1.1 table

**Backlog** (in rough order; each gets planned before it starts)

1. Thumbnails, a photo-style viewer and multi-select (save several to Photos, download a selection as a ZIP): the first candidate after M9
2. Resumable uploads (files over 4 GB, surviving a lock or a reload): if large uploads still fail after M7
3. Folder upload from the iPhone: the iPhone check confirmed the picker can choose a whole folder, so it needs a client-side `mkdir` per folder plus the existing upload
4. Chat search with link / photo / file filters
5. Connection troubleshooting in the Add device window
6. ✅ A release process (done 2026-10-20): pushing a tag `vX.Y.Z` on `main` runs `.github/workflows/release.yml`, which reuses the CI jobs (tests, the Windows installer and its install check), builds and checks the ad-hoc-signed Mac app, and publishes a GitHub Release with both installers and `SHA256SUMS.txt`; versions below 1.0 are pre-releases. `src-tauri/Cargo.toml` is the only place that holds the version. Settings has a **查看新版本** (Check for new version) link that only opens the Releases page in the browser; the app itself never contacts the internet. Steps in `docs/releasing.md`
7. Send to the phone from the Finder / File Explorer (drop on the Dock icon, Open With, SendTo)
8. Computer-to-computer transfers (Mac ↔ Windows)
9. Folder permissions per device
10. CI supply-chain hardening: pin actions to commit SHAs, pin the Rust toolchain, a cargo-deny / audit job, checksums on artifacts; Dependabot if you approve it
11. A Windows ARM64 build; code signing and notarization

**Not doing** (platform limits, or conflicts with the LAN-only and security rules)

- Appearing in the iOS share sheet as a web page: WebKit on iOS does not support Web Share Target (the M9 Shortcut covers this)
- Putting received text on the iPhone clipboard automatically: Safari allows clipboard writes only during a tap
- Uploading while Safari is in the background: iOS suspends hidden pages and has no Background Fetch or Sync for the web
- Automatic update checks, telemetry, or a GitHub token embedded in the app: the app would contact the internet
- Approving a pairing from a notification: the confirmation codes must be compared in the window
- Watching the computer's clipboard continuously: privacy
- A separate "copy image" button: the share sheet behind **存储到相册** (Save to Photos) already offers Copy and WeChat (4.1.2)

**Long term**: native iOS app (see section 11)

## 13. Test plan

- **Automated tests** (run in CI on both macOS and Windows):
  - Rust unit tests
  - API integration tests: start a real server in a temporary directory; cover pairing, auth, messages, files, Range and all kinds of error cases
  - Frontend type checking; frontend unit tests (vitest, from M7)
- **iOS Simulator** (I can run it directly on this Mac): at the end of each milestone, walk through the iPhone web flows and take screenshots to check them
- **Real iPhone** (needs your help): once each at M0, M4 and M5; then the 15-minute iPhone check (done 2026-10-20), and once at the end of M7 and M9
- **Windows PC** (later, with the real iPhone): the "Windows testing, later" list in section 12, in one pass of about 1.5 hours
- **Security self-review checklist**:
  - Access without a cookie
  - Forged Origin
  - All kinds of path-escape variants
  - Previewing HTML files
  - Brute-forcing the pairing code
  - Requests from internet IPs (simulated in unit tests)

## 14. Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Certificate warning on the iPhone's first visit | User experience | Guide the user to install the certificate (verified to remove the warning completely) |
| Connection drops after the iPhone goes to the background or locks | Delayed messages; large file uploads interrupted | SSE reconnects automatically and fetches what was missed; tell the user to keep the page open during uploads; M7 keeps the screen on and retries uploads after the network returns; resumable uploads are on the backlog |
| Uploaded photos are transcoded (HEIC → JPEG) | You do not get the original | Confirmed on a real device that a web page cannot avoid this; on the first upload, suggest sending originals through the Files app (see 4.1.1) |
| The computer's IP changes | The Home Screen bookmark stops working | The certificate is re-signed automatically; just scan the QR code again; M9 adds connecting by the `.local` hostname |
| No real Windows device | Windows problems are found late | CI builds, installs and tests every change on Windows; all hands-on checks are collected in one list and run once a Windows PC is available |
| Network environment (AP isolation, guest networks, VPN, Windows set to "Public network") | Cannot connect | The Settings page gives troubleshooting tips; explained in the docs |
| The computer idle-sleeps during a transfer | The transfer fails | Keep the computer awake while a transfer is active (M7) |
| Port already in use | The address in the QR code changes | Move to the next port automatically, and show the actual port on the Settings page |
| The two platforms use different WebViews (WKWebView on macOS, WebView2 on Windows) | UI compatibility problems | Test on both platforms; use only standard web features |

## 15. Things you need to prepare or confirm

1. **GitHub**: a GitHub account and a private repository (any time before M6 is fine; needed to build the Windows version)
2. **Windows testing** (changed 2026-10-20): no VM. Hands-on Windows testing waits until you have a Windows PC; then the "Windows testing, later" list in section 12 is run in one pass, with the iPhone on the same Wi-Fi
3. **Real-device help**: at M0, M4 and M5, connect the iPhone and the Mac to the same Wi-Fi and help test for a few minutes; then the 15-minute iPhone check, and one test session at the end of each milestone that needs one (times are listed under each milestone)
4. **Name**: is the working name "LanShare" OK? Is `~/LanShare` OK as the default shared folder location?
5. **Decisions for later milestones**, asked when each one starts: the global shortcut and its target device (M8), making `.local` the default QR code and the Shortcuts details (M9), the update policy and Dependabot (backlog), and approval of each new dependency

## References

- Apple membership comparison (free account limits): https://developer.apple.com/support/compare-memberships/
- Apple Developer app enrollment requirements: https://developer.apple.com/support/app-account/
- Apple Developer enrollment details (annual fee charged in local currency): https://developer.apple.com/support/enrollment/
- Forum thread from an Australian user about the amount charged: https://developer.apple.com/forums/thread/775152
- GitHub Actions billing: https://docs.github.com/en/billing/concepts/product-billing/github-actions
- tauri-action: https://github.com/tauri-apps/tauri-action
- Tauri v2 guide to building on GitHub: https://v2.tauri.app/distribute/pipelines/github
- WebSocket problems in Safari with self-signed certificates: https://bugs.webkit.org/show_bug.cgi?id=158345, https://developer.apple.com/forums/thread/792842, https://cockpit-project.org/running/safari
