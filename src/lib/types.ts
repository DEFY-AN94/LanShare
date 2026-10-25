// Wire types shared with the Rust server (src-tauri/src/core/model.rs).
// All timestamps are milliseconds since the Unix epoch.

export type MessageKind = "text" | "file";

export interface FileRef {
  path: string;
  name: string;
  size: number;
  mime: string;
  exists: boolean;
}

export interface Message {
  id: number;
  device_id: string;
  from_host: boolean;
  client_id: string | null;
  kind: MessageKind;
  text: string | null;
  file: FileRef | null;
  created_at: number;
}

export interface MessagePage {
  messages: Message[];
  has_more: boolean;
}

export interface Me {
  kind: "host" | "device";
  device_id: string | null;
  device_name: string | null;
  host_name: string;
  can_write: boolean;
  paused: boolean;
}

export interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: number;
}

export interface DirListing {
  path: string;
  entries: FileEntry[];
  can_write: boolean;
}

export interface Device {
  id: string;
  name: string;
  can_write: boolean;
  created_at: number;
  last_seen_at: number | null;
  online: boolean;
  /** Paired under an earlier root certificate: it must pair again, and its history can only be read. */
  needs_repair: boolean;
  last_message: Message | null;
  /** Messages from this device the computer has not read yet. */
  unread: number;
}

export interface Address {
  ip: string;
  iface: string;
}

export interface PairingUrl extends Address {
  url: string;
  qr_svg: string;
}

export interface PairingSession {
  code: string;
  expires_at: number;
  urls: PairingUrl[];
  ca_fingerprint: string;
  network_warning: string | null;
}

/** Step 1 of 添加设备: where a phone installs the root certificate. */
export interface CertSetup {
  urls: PairingUrl[];
  ca_name: string;
  ca_fingerprint: string;
  network_warning: string | null;
  paused: boolean;
}

export interface PairRequest {
  id: string;
  name: string;
  confirm_code: string;
  ip: string;
  created_at: number;
}

export interface HostOverview {
  host_name: string;
  port: number;
  addresses: Address[];
  shared_root: string;
  /** Why the shared folder cannot be used right now: its disk is not connected, or the folder was moved or deleted. */
  shared_root_unavailable: "disk" | "folder" | null;
  /** The folder chosen in Settings, while it is unavailable and `shared_root` stands in for it. */
  preferred_root: string | null;
  ca_fingerprint: string;
  /** The local CA's name, as iPhone settings list it. */
  ca_name: string;
  paused: boolean;
  version: string;
  network_warning: string | null;
  /** Automatic chat cleanup: messages older than this many days are deleted; 0 = off. */
  chat_retention_days: number;
  /** Debug-level logging; resets when LanShare restarts. */
  verbose_logging: boolean;
  platform: "macos" | "windows" | "linux" | string;
  /** Whether the system shows LanShare's notifications. */
  notifications: "allowed" | "denied" | "notDetermined" | "unavailable" | "unknown";
  notify_hide_text: boolean;
  /** The global shortcut that sends the clipboard (e.g. "Control+Alt+Super+KeyV"), if set. */
  clipboard_shortcut: string | null;
  /** Why that shortcut could not be registered when LanShare started. */
  clipboard_shortcut_error: string | null;
  /** The device the clipboard goes to; null is the most recent conversation. */
  clipboard_target: string | null;
}

/** Launch at login, as macOS reports it (macOS 13+; "unsupported" elsewhere). */
export type LoginItemState = "unsupported" | "notInstalled" | "off" | "on" | "needsApproval";

export interface LoginItemInfo {
  state: LoginItemState;
  /** Offer it once, after the first pairing. */
  suggest?: boolean;
}

export interface PairRequestResponse {
  request_id: string;
  confirm_code: string;
  host_name: string;
}

export type PairStatus = "pending" | "approved" | "rejected" | "expired";
