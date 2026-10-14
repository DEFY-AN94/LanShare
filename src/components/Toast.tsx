import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from "react";

type Kind = "info" | "success" | "error";
interface ToastItem {
  id: number;
  text: string;
  kind: Kind;
}

const ToastContext = createContext<(text: string, kind?: Kind) => void>(() => {});

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const nextId = useRef(1);

  const show = useCallback((text: string, kind: Kind = "info") => {
    const id = nextId.current++;
    // Collapse repeats of the same text (e.g. tapping copy repeatedly).
    setItems((prev) => [...prev.filter((t) => t.text !== text), { id, text, kind }].slice(-3));
    window.setTimeout(() => setItems((prev) => prev.filter((t) => t.id !== id)), kind === "error" ? 3500 : 1800);
  }, []);

  return (
    <ToastContext.Provider value={show}>
      {children}
      <div className="toasts" role="status" aria-live="polite">
        {items.map((t) => (
          <div key={t.id} className={`toast toast-${t.kind}`}>
            {t.text}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast() {
  return useContext(ToastContext);
}
