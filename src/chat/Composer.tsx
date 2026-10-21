import { useEffect, useRef, useState } from "react";
import { Icon } from "../components/Icon";
import { usePhotoTip } from "../components/PhotoTip";
import { useToast } from "../components/Toast";
import { loadLocal, removeLocal, saveLocal } from "../lib/storage";
import { openPicker } from "../lib/wakeLock";

interface Props {
  disabled?: boolean;
  /** Not connected: messages are queued and sent once the connection is back. */
  offline?: boolean;
  placeholder?: string;
  /** Which conversation's draft this is. */
  draftKey: string;
  /** Keep the draft in localStorage too (the phone), not only in memory. */
  persistDraft?: boolean;
  onSendText: (text: string) => void;
  onSendFiles: (files: File[]) => void;
}

const MAX_TEXT = 100_000;
const coarsePointer = () => window.matchMedia("(pointer: coarse)").matches;
/** Drafts survive switching tabs or conversations. */
const drafts = new Map<string, string>();
const storageKey = (key: string) => `draft.${key}`;

function initialDraft(key: string, persist: boolean): string {
  return drafts.get(key) ?? (persist ? loadLocal<string>(storageKey(key), "") : "");
}

export function Composer({ disabled, offline, placeholder = "输入消息", draftKey, persistDraft = false, onSendText, onSendFiles }: Props) {
  const [text, setText] = useState(() => initialDraft(draftKey, persistDraft));
  const ref = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const photoTip = usePhotoTip(() => openPicker(fileRef.current));
  const toast = useToast();
  const canPaste = typeof navigator.clipboard?.readText === "function";

  // Keep the draft: in memory at once, on the phone also in storage (briefly
  // delayed while typing, at once when the composer goes away).
  useEffect(() => {
    drafts.set(draftKey, text);
    if (!persistDraft) return;
    const save = () => (text ? saveLocal(storageKey(draftKey), text) : removeLocal(storageKey(draftKey)));
    const t = window.setTimeout(save, 300);
    // iOS may end the page soon after it is hidden.
    const onHide = () => document.visibilityState === "hidden" && save();
    document.addEventListener("visibilitychange", onHide);
    window.addEventListener("pagehide", save);
    return () => {
      window.clearTimeout(t);
      document.removeEventListener("visibilitychange", onHide);
      window.removeEventListener("pagehide", save);
      save();
    };
  }, [text, draftKey, persistDraft]);

  // Auto-grow up to ~6 lines.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [text]);

  const send = () => {
    const value = text.trim();
    if (!value || disabled) return;
    if (value.length > MAX_TEXT) {
      toast("消息太长了，请改用文件发送", "error");
      return;
    }
    onSendText(value);
    setText("");
    // Keep the keyboard open on phones for quick follow-ups.
    ref.current?.focus();
  };

  const pasteFromClipboard = async () => {
    try {
      const value = await navigator.clipboard.readText();
      if (!value) return toast("剪贴板是空的");
      setText((t) => (t ? `${t}${value}` : value));
      ref.current?.focus();
    } catch {
      toast("无法读取剪贴板，请长按输入框粘贴", "error");
    }
  };

  return (
    <div className="composer">
      <button className="icon-btn" aria-label="发送文件" title="发送文件" disabled={disabled} onClick={photoTip.open}>
        <Icon name="paperclip" />
      </button>
      <input
        ref={fileRef}
        type="file"
        multiple
        hidden
        onChange={(e) => {
          const files = Array.from(e.target.files ?? []);
          e.target.value = "";
          if (files.length) onSendFiles(files);
        }}
      />
      <textarea
        ref={ref}
        className="composer-input"
        rows={1}
        value={text}
        placeholder={disabled ? "已断开连接" : offline ? "未连接：消息会在连上后发出" : placeholder}
        disabled={disabled}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          // Enter sends on desktop; IME composition (Chinese input) must not send.
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing && e.keyCode !== 229 && !coarsePointer()) {
            e.preventDefault();
            send();
          }
        }}
        onPaste={(e) => {
          const files = Array.from(e.clipboardData.files);
          if (files.length) {
            e.preventDefault();
            onSendFiles(files);
          }
        }}
      />
      {!text && canPaste && coarsePointer() ? (
        <button className="icon-btn" aria-label="粘贴" title="粘贴剪贴板" disabled={disabled} onClick={pasteFromClipboard}>
          <Icon name="clipboard" />
        </button>
      ) : (
        <button className="send-btn" aria-label="发送" disabled={disabled || !text.trim()} onClick={send}>
          <Icon name="send" size={18} strokeWidth={2.4} />
        </button>
      )}
      {photoTip.element}
    </div>
  );
}
