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
  last_message: Message | null;
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
  ca_fingerprint: string;
  /** The local CA's name, as iPhone settings list it. */
  ca_name: string;
  paused: boolean;
  version: string;
  network_warning: string | null;
}

export interface PairRequestResponse {
  request_id: string;
  confirm_code: string;
  host_name: string;
}

export type PairStatus = "pending" | "approved" | "rejected" | "expired";
