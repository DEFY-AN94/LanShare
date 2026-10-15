# LanShare project plan

> A messaging and file-transfer tool for iPhone ↔ computer (macOS / Windows) on the local network. LAN only; nothing goes through any cloud.
>
> Document date: 2026-10-15 · Status: Mac MVP complete (M0–M5, branch `mac-mvp`; for M5 acceptance testing on a real device, see 4.1.1); M6 (Windows) merged into `main` (PR #1); the installer still needs testing on a real Windows PC · "LanShare" is a working name and can be changed

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
| Windows version | GitHub Actions builds the packages automatically; test in a Windows VM on the Mac | No Windows computer at hand |
| Native app | Long term; an Australian developer account is an option | Feasible; see section 11 |
| Development order (changed 2026-10-15) | Start with local Git and build the Mac version first; GitHub Actions and the Windows version come after you create the GitHub repository | The GitHub repository will be created later |

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
| Screen locked during upload | ✅ Locked the screen right after a ~600 MB video upload started (LAN at 30–40 MB/s); after unlocking, the upload had finished. A longer lock can still get interrupted by iOS (resumable uploads are in M7) |
| Add to Home Screen | ✅ The Home Screen app reuses Safari's pairing credential and opens straight into the chat |
| Without the certificate installed | The Home Screen app shows a certificate warning on every launch; tapping "Visit" gets through |
| Home Screen icon | ✅ (after fix) At first it was a gray letter L, while the icon in the share sheet looked fine. Cause: "Add to Home Screen" downloads apple-touch-icon in a separate system process (SafariViewService), which does not reuse Safari's "continue to visit" choice. In the Simulator it reproduces while the certificate is untrusted and works once it is trusted; on a real device it still failed with the certificate trusted — the computer's log shows some of the phone's background connections rejecting the certificate with `CertificateUnknown`, while Safari and the Home Screen app themselves show no warning. Fix: at build time, embed the icon in the page as a data: URL (`vite.config.ts`), so no download is needed; re-tested on a real device and it works. The icons were also changed to standard sizes (180 / 192 / 512) |
| Certificate profile install | ✅ Fingerprint location: Settings › Profile Downloaded › More Details › certificate name › "Fingerprints" at the very bottom; the SHA-256 matches the one on the computer. During install, iOS shows "will change settings on your iPhone", "Not Verified" and "Unmanaged Root Certificate" — generic warnings shown for every profile; this profile contains only one certificate. Once it is installed and enabled in "Certificate Trust Settings", no warnings remain |
| Certificate name | ✅ (after fix) Every root certificate used to be named "LanShare Local CA (Mac)": each dev run, the release build and every **重置证书** (Reset certificate) created its own, and they looked identical in "VPN & Device Management" and "Certificate Trust Settings", so there was no way to tell which one was old. New certificate names now end with a 6-character ID (taken from the certificate's key identifier, e.g. "LanShare Local CA (Mac) 2DD31B"), and both **设置 › 安全** (Settings › Security) in the desktop app and the iPhone install guide show the current certificate's full name. Old certificates that are already installed are not regenerated just to rename them; their names stay the same |
| The `.local` name in the certificate | ✅ (after fix) It used to come from the hostname, which on this Mac is `mac.local` (a hostname assigned by the router), while Bonjour actually answers to `MyMac.local`, so `.local` access in M7 would fail. On macOS it now uses the Bonjour local hostname (System Settings › General › Sharing › Local hostname) and falls back to the hostname only when that is unavailable; after a rename, the server certificate is re-signed automatically |
| Computer name | ✅ (after fix) The computer name in the phone's top bar, on the pairing page and in new certificate names used to come from the hostname, which shows as "Mac" on this Mac. On macOS it now uses "Computer name" (System Settings › General › Sharing, i.e. `scutil --get ComputerName`, here "MyMac") and falls back to the hostname only when that is unavailable; Windows is unchanged. A rename takes effect after restarting LanShare; existing root certificates are not regenerated because of it |

### 4.2 Windows version (no Windows machine at hand)

- **Packaging**: the official `tauri-apps/tauri-action@v1` builds on a GitHub-hosted Windows machine (`windows-latest`) and produces `.msi` / `.exe` installers. They can simply be downloaded as build artifacts; there is no need to publish a Release.
- **Cost**: private repositories get 2,000 free minutes per month; Windows machines cost more per minute than Linux, so the quota runs out faster. A cold Rust build takes about 10–20 minutes, which is enough for a personal project. **With no payment method attached, nothing is charged**; when the quota runs out, runs just pause. Public repositories are completely free.
- **Testing**:
  1. Automated tests: every push runs Rust unit tests and API tests on Windows (path rules, file reads and writes, API)
  2. Manual tests: install a Windows 11 ARM VM on this Mac (UTM free / VMware Fusion free / Parallels paid), install the package CI builds and use it for real. The x64 installer runs under emulation on Windows on ARM; CI can also build an extra ARM64 version
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
- Later (M7): resumable transfers, thumbnails, multi-select and download as an archive

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
| Privacy | No accounts, no servers, no data reporting; logs do not record message content; chat history is stored only on this computer and can be cleared, or set to clean up automatically on a schedule |
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
  - `config.json` (shared folder path, port)
  - `tls/` (certificates and private keys)
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

Each milestone has a result that can be verified in acceptance testing. Finish one before starting the next.

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
- Status: merged into `main` through PR #1 (2026-10-15). CI on Windows passes clippy, all Rust tests and the API end-to-end test, and builds the installer. **Not accepted yet: the installer has not been run on a real Windows PC.**
- Done (2026-10-15; the code is on GitHub at DEFY-AN94/LanShare):
  - The installer (NSIS, installs to Program Files) adds a firewall rule automatically: it allows only Private networks and the local subnet; uninstalling removes it
  - At runtime, detect when Windows is set to "Public network", and prompt on the Settings page and in the Add device window to switch it to a Private network
  - Before deleting, check the drive type, and refuse to delete on USB flash drives / network shares (they have no Recycle Bin); run Recycle Bin operations on a separate thread to avoid COM initialization conflicts
  - VPN adapters (point-to-point links, common VPN names) no longer appear in the QR code address
  - Upload handlers read any request body they left unread before answering; otherwise the connection reset on Windows discarded the response, and a retried upload that had gone through looked failed
  - All `cfg(windows)` code (hidden attribute, Zone.Identifier, long paths, UNC paths) compiles in CI, and all Rust tests pass on Windows
  - CI: on Windows, run clippy, all Rust tests and the API end-to-end test, and build the Windows installer; the end-to-end test fails the job when any check fails (macOS tests run only on PRs)
- GitHub Actions: on every push to `main` and on every PR, run the tests and package the NSIS `.exe` installer as the build artifact `LanShare-windows-x64`
- Still to test on a real Windows PC (or the VM):
  - Install and uninstall: the SmartScreen and administrator prompts; the firewall rule is added (Private networks and the local subnet only) and removed again
  - The "Public network" warning on the Settings page and in the Add device window
  - Delete refusal on USB flash drives / network shares
  - With a full-tunnel VPN connected, the QR code still shows the LAN address
  - The app window, tray icon and notifications; files from the phone get Zone.Identifier
- Install and test in a Windows 11 ARM virtual machine on the Mac
- Acceptance: Actions produces the installer; after installing on Windows, it can complete pairing and transfers with an iPhone

**M7 Enhancements (pick as needed)**

- Resumable transfers for large files, image thumbnails, multi-select and download as an archive
- Access through `computer-name.local`, so it still connects after the IP changes (the server certificate already includes the Bonjour name, e.g. `mymac.local`)
- Computer-to-computer connections (Mac ↔ Windows)
- A global shortcut in the desktop app: "send clipboard to phone"
- Automatic chat history cleanup, launch at login

**Long term**: native iOS app (see section 11)

## 13. Test plan

- **Automated tests** (run in CI on both macOS and Windows):
  - Rust unit tests
  - API integration tests: start a real server in a temporary directory; cover pairing, auth, messages, files, Range and all kinds of error cases
  - Frontend type checking
- **iOS Simulator** (I can run it directly on this Mac): at the end of each milestone, walk through the iPhone web flows and take screenshots to check them
- **Real iPhone** (needs your help): once each at M0, M4 and M5
- **Windows virtual machine**: M6
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
| Connection drops after the iPhone goes to the background or locks | Delayed messages; large file uploads interrupted | SSE reconnects automatically and fetches what was missed; tell the user to keep the page open during uploads; resumable transfers in M7 |
| Uploaded photos are transcoded (HEIC → JPEG) | You do not get the original | Confirmed on a real device that a web page cannot avoid this; on the first upload, suggest sending originals through the Files app (see 4.1.1) |
| The computer's IP changes | The Home Screen bookmark stops working | The certificate is re-signed automatically; just scan the QR code again; M7 supports the `.local` hostname |
| No real Windows device | Windows problems are found late | CI automated tests + virtual machine tests; confirm on real hardware once a Windows PC is available |
| Network environment (AP isolation, guest networks, VPN, Windows set to "Public network") | Cannot connect | The Settings page gives troubleshooting tips; explained in the docs |
| Port already in use | The address in the QR code changes | Move to the next port automatically, and show the actual port on the Settings page |
| The two platforms use different WebViews (WKWebView on macOS, WebView2 on Windows) | UI compatibility problems | Test on both platforms; use only standard web features |

## 15. Things you need to prepare or confirm

1. **GitHub**: a GitHub account and a private repository (any time before M6 is fine; needed to build the Windows version)
2. **Windows testing**: is it OK to install a Windows 11 ARM virtual machine on the Mac (UTM, free), or should testing wait until you have a Windows PC?
3. **Real-device help**: at M0, M4 and M5, connect the iPhone and the Mac to the same Wi-Fi and help test for a few minutes
4. **Name**: is the working name "LanShare" OK? Is `~/LanShare` OK as the default shared folder location?

## References

- Apple membership comparison (free account limits): https://developer.apple.com/support/compare-memberships/
- Apple Developer app enrollment requirements: https://developer.apple.com/support/app-account/
- Apple Developer enrollment details (annual fee charged in local currency): https://developer.apple.com/support/enrollment/
- Forum thread from an Australian user about the amount charged: https://developer.apple.com/forums/thread/775152
- GitHub Actions billing: https://docs.github.com/en/billing/concepts/product-billing/github-actions
- tauri-action: https://github.com/tauri-apps/tauri-action
- Tauri v2 guide to building on GitHub: https://v2.tauri.app/distribute/pipelines/github
- WebSocket problems in Safari with self-signed certificates: https://bugs.webkit.org/show_bug.cgi?id=158345, https://developer.apple.com/forums/thread/792842, https://cockpit-project.org/running/safari
