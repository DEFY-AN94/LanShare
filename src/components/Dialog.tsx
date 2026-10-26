// Promise-based confirm/prompt dialogs. We avoid window.confirm/prompt
// because the desktop WebView does not implement them consistently.

import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { useBackdropClose } from "../lib/useBackdropClose";

interface ConfirmOptions {
  title: string;
  message?: ReactNode;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
}

interface PromptOptions {
  title: string;
  label?: string;
  initial?: string;
  placeholder?: string;
  confirmText?: string;
  /** Pre-select the name part before the extension (for renames). */
  selectStem?: boolean;
}

type Pending =
  | { type: "confirm"; opts: ConfirmOptions; resolve: (v: boolean) => void }
  | { type: "prompt"; opts: PromptOptions; resolve: (v: string | null) => void };

interface DialogApi {
  confirm: (opts: ConfirmOptions) => Promise<boolean>;
  prompt: (opts: PromptOptions) => Promise<string | null>;
}

const DialogContext = createContext<DialogApi>({
  confirm: async () => false,
  prompt: async () => null,
});

export function DialogProvider({ children }: { children: ReactNode }) {
  const [pending, setPending] = useState<Pending | null>(null);

  const confirm = useCallback(
    (opts: ConfirmOptions) => new Promise<boolean>((resolve) => setPending({ type: "confirm", opts, resolve })),
    [],
  );
  const prompt = useCallback(
    (opts: PromptOptions) => new Promise<string | null>((resolve) => setPending({ type: "prompt", opts, resolve })),
    [],
  );

  const close = (value: boolean | string | null) => {
    if (!pending) return;
    if (pending.type === "confirm") pending.resolve(value === true);
    else pending.resolve(typeof value === "string" ? value : null);
    setPending(null);
  };

  return (
    <DialogContext.Provider value={{ confirm, prompt }}>
      {children}
      {pending && <DialogView pending={pending} onClose={close} />}
    </DialogContext.Provider>
  );
}

function DialogView({ pending, onClose }: { pending: Pending; onClose: (v: boolean | string | null) => void }) {
  const [value, setValue] = useState(pending.type === "prompt" ? (pending.opts.initial ?? "") : "");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const input = inputRef.current;
    if (!input || pending.type !== "prompt") return;
    input.focus();
    const dot = input.value.lastIndexOf(".");
    if (pending.opts.selectStem && dot > 0) input.setSelectionRange(0, dot);
    else input.select();
  }, [pending]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose(pending.type === "confirm" ? false : null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [pending, onClose]);

  const opts = pending.opts;
  const isPrompt = pending.type === "prompt";
  const danger = pending.type === "confirm" && pending.opts.danger;
  const submit = () => onClose(isPrompt ? value.trim() || null : true);
  const backdrop = useBackdropClose(() => onClose(isPrompt ? null : false));

  return (
    <div className="modal-backdrop" {...backdrop}>
      <form
        className="modal dialog"
        role="dialog"
        aria-modal="true"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <h2 className="dialog-title">{opts.title}</h2>
        {pending.type === "confirm" && pending.opts.message && <div className="dialog-message">{pending.opts.message}</div>}
        {isPrompt && (
          <label className="field">
            {(opts as PromptOptions).label && <span className="field-label">{(opts as PromptOptions).label}</span>}
            <input
              ref={inputRef}
              className="input"
              value={value}
              placeholder={(opts as PromptOptions).placeholder}
              onChange={(e) => setValue(e.target.value)}
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
            />
          </label>
        )}
        <div className="dialog-actions">
          <button type="button" className="btn" onClick={() => onClose(isPrompt ? null : false)}>
            {pending.type === "confirm" ? (pending.opts.cancelText ?? "取消") : "取消"}
          </button>
          <button type="submit" className={`btn ${danger ? "btn-danger" : "btn-primary"}`} disabled={isPrompt && !value.trim()}>
            {opts.confirmText ?? "确定"}
          </button>
        </div>
      </form>
    </div>
  );
}

export function useDialog() {
  return useContext(DialogContext);
}
