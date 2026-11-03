// Promise-based confirm/prompt dialogs. We avoid window.confirm/prompt
// because the desktop WebView does not implement them consistently.

import { createContext, useCallback, useContext, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { useBackdropClose } from "../lib/useBackdropClose";
import { useModalFocus } from "../lib/useModalFocus";

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

type Request =
  | { type: "confirm"; opts: ConfirmOptions; resolve: (v: boolean) => void }
  | { type: "prompt"; opts: PromptOptions; resolve: (v: string | null) => void };
type Pending = Request & { id: number };

function settle(p: Pending, value: boolean | string | null) {
  if (p.type === "confirm") p.resolve(value === true);
  else p.resolve(typeof value === "string" ? value : null);
}

interface DialogApi {
  confirm: (opts: ConfirmOptions) => Promise<boolean>;
  prompt: (opts: PromptOptions) => Promise<string | null>;
}

const DialogContext = createContext<DialogApi>({
  confirm: async () => false,
  prompt: async () => null,
});

export function DialogProvider({ children }: { children: ReactNode }) {
  // One dialog at a time: a request made while one is open waits its turn, so
  // neither is answered before the user has seen it.
  const [queue, setQueue] = useState<Pending[]>([]);
  // Outside the state updater, which StrictMode runs twice.
  const nextId = useRef(1);

  const show = useCallback((r: Request) => {
    const p = { ...r, id: nextId.current++ };
    setQueue((q) => [...q, p]);
  }, []);
  const confirm = useCallback((opts: ConfirmOptions) => new Promise<boolean>((resolve) => show({ type: "confirm", opts, resolve })), [show]);
  const prompt = useCallback((opts: PromptOptions) => new Promise<string | null>((resolve) => show({ type: "prompt", opts, resolve })), [show]);

  const close = useCallback((p: Pending, value: boolean | string | null) => {
    settle(p, value);
    setQueue((q) => q.filter((x) => x !== p));
  }, []);

  const pending = queue[0];
  return (
    <DialogContext.Provider value={{ confirm, prompt }}>
      {children}
      {pending && <DialogView key={pending.id} pending={pending} onClose={(v) => close(pending, v)} />}
    </DialogContext.Provider>
  );
}

function DialogView({ pending, onClose }: { pending: Pending; onClose: (v: boolean | string | null) => void }) {
  const [value, setValue] = useState(pending.type === "prompt" ? (pending.opts.initial ?? "") : "");
  const ref = useRef<HTMLFormElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const messageId = useId();
  const isPrompt = pending.type === "prompt";
  const danger = pending.type === "confirm" && pending.opts.danger;
  // A delete cannot be undone, so Enter there cancels unless the user moves to
  // the button. This runs before the prompt's focus effect below, so it saves
  // what had focus before the dialog opened.
  useModalFocus(ref, { initial: isPrompt ? inputRef : danger ? cancelRef : confirmRef, onEscape: () => onClose(isPrompt ? null : false) });

  useEffect(() => {
    const input = inputRef.current;
    if (!input || pending.type !== "prompt") return;
    input.focus();
    const dot = input.value.lastIndexOf(".");
    if (pending.opts.selectStem && dot > 0) input.setSelectionRange(0, dot);
    else input.select();
  }, [pending]);

  const opts = pending.opts;
  const message = pending.type === "confirm" ? pending.opts.message : undefined;
  const submit = () => onClose(isPrompt ? value.trim() || null : true);
  const backdrop = useBackdropClose(() => onClose(isPrompt ? null : false));

  return (
    <div className="modal-backdrop" {...backdrop}>
      <form
        ref={ref}
        className="modal dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={message ? messageId : undefined}
        tabIndex={-1}
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <h2 className="dialog-title" id={titleId}>
          {opts.title}
        </h2>
        {message && (
          <div className="dialog-message" id={messageId}>
            {message}
          </div>
        )}
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
          <button ref={cancelRef} type="button" className="btn" onClick={() => onClose(isPrompt ? null : false)}>
            {pending.type === "confirm" ? (pending.opts.cancelText ?? "取消") : "取消"}
          </button>
          <button ref={confirmRef} type="submit" className={`btn ${danger ? "btn-danger" : "btn-primary"}`} disabled={isPrompt && !value.trim()}>
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
