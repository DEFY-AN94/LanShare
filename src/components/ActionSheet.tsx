import { useEffect } from "react";
import { Icon } from "./Icon";
import { useBackdropClose } from "../lib/useBackdropClose";

export interface SheetAction {
  label: string;
  icon?: string;
  danger?: boolean;
  onSelect: () => void;
}

interface Props {
  title?: string;
  subtitle?: string;
  actions: SheetAction[];
  onClose: () => void;
}

/** Bottom sheet on phones, small centered menu on desktop (see .sheet in CSS). */
export function ActionSheet({ title, subtitle, actions, onClose }: Props) {
  const backdrop = useBackdropClose(onClose);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="modal-backdrop sheet-backdrop" {...backdrop}>
      <div className="sheet" role="menu">
        {(title || subtitle) && (
          <div className="sheet-header">
            {title && <div className="sheet-title ellipsis">{title}</div>}
            {subtitle && <div className="sheet-subtitle">{subtitle}</div>}
          </div>
        )}
        {actions.map((a) => (
          <button
            key={a.label}
            role="menuitem"
            className={`sheet-item ${a.danger ? "text-danger" : ""}`}
            onClick={() => {
              onClose();
              a.onSelect();
            }}
          >
            {a.icon && <Icon name={a.icon} size={18} />}
            <span>{a.label}</span>
          </button>
        ))}
        <button className="sheet-item sheet-cancel" onClick={onClose}>
          取消
        </button>
      </div>
    </div>
  );
}
