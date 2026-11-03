import { useId, useRef } from "react";
import { Icon } from "./Icon";
import { useBackdropClose } from "../lib/useBackdropClose";
import { useModalFocus } from "../lib/useModalFocus";

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
  const ref = useRef<HTMLDivElement>(null);
  const first = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  // Focus starts on the first action that deletes nothing, else on 取消.
  const firstSafe = actions.findIndex((a) => !a.danger);
  useModalFocus(ref, { initial: first, onEscape: onClose });

  return (
    <div className="modal-backdrop sheet-backdrop" {...backdrop}>
      <div
        ref={ref}
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby={title ? titleId : undefined}
        aria-label={title ? undefined : "更多操作"}
        tabIndex={-1}
      >
        {(title || subtitle) && (
          <div className="sheet-header">
            {title && (
              <div className="sheet-title ellipsis" id={titleId}>
                {title}
              </div>
            )}
            {subtitle && <div className="sheet-subtitle">{subtitle}</div>}
          </div>
        )}
        {actions.map((a, i) => (
          <button
            key={a.label}
            ref={i === firstSafe ? first : undefined}
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
        <button ref={firstSafe < 0 ? first : undefined} className="sheet-item sheet-cancel" onClick={onClose}>
          取消
        </button>
      </div>
    </div>
  );
}
