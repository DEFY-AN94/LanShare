// Keyboard focus for dialogs and sheets: it moves in when one opens, Tab and
// Shift+Tab stay inside, Escape closes the one on top, and focus goes back to
// where it was when it closes. Without this, Enter after clicking 移除 pressed
// 移除 again behind the confirm (WebView2 focuses a clicked button), and Tab
// walked the page behind it.

import { useEffect, useRef, type RefObject } from "react";
import { coarsePointer } from "./pointer";

// Tab is handled here, so only these can be reached in a dialog. There is no
// visibility check: a control that should not be reached is left unrendered,
// not hidden with CSS.
const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), audio[controls], video[controls], [tabindex]:not([tabindex="-1"])';

interface OpenDialog {
  el: HTMLElement;
  onKey: (e: KeyboardEvent) => void;
}

const open: OpenDialog[] = [];

/**
 * Dialogs are drawn in page order (the file viewer, at z-index 40, always comes
 * before the save sheet it opens), so the one latest in the page is on top. A
 * modal added at another z-index must keep that order. One already taken out of
 * the page, whose effect has not been cleaned up yet, no longer counts.
 */
function topModal(): OpenDialog | undefined {
  return open.reduce<OpenDialog | undefined>((a, b) => (!b.el.isConnected ? a : !a || a.el.compareDocumentPosition(b.el) & Node.DOCUMENT_POSITION_FOLLOWING ? b : a), undefined);
}

// One listener for all dialogs, so a key reaches only the dialog on top, even
// when closing that one lets React commit before a second listener would run.
function onKeyDown(e: KeyboardEvent) {
  topModal()?.onKey(e);
}

function cycle(el: HTMLElement, backwards: boolean) {
  const items = Array.from(el.querySelectorAll<HTMLElement>(FOCUSABLE));
  if (!items.length) return el.focus();
  const i = items.indexOf(document.activeElement as HTMLElement);
  const next = backwards ? (i <= 0 ? items.length - 1 : i - 1) : i === items.length - 1 ? 0 : i + 1;
  items[next].focus();
}

/**
 * `ref` is the dialog element, which needs role="dialog" (or "alertdialog"),
 * aria-modal="true" and tabIndex={-1}. `initial` gets focus when it opens, else
 * the dialog itself; `onEscape` closes it, and without it Escape does nothing.
 */
export function useModalFocus(ref: RefObject<HTMLElement | null>, opts: { initial?: RefObject<HTMLElement | null>; onEscape?: () => void } = {}) {
  const { initial } = opts;
  const onEscape = useRef(opts.onEscape);
  onEscape.current = opts.onEscape;

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const back = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // On a touch screen a text field keeps its focus: taking it would close the
    // iPhone keyboard, and giving it back would open it again.
    const keep = coarsePointer() && !!back?.matches("input, textarea, select");
    const self: OpenDialog = {
      el,
      onKey: (e) => {
        if (e.isComposing || e.keyCode === 229) return;
        if (e.key === "Escape" && onEscape.current) {
          e.preventDefault();
          onEscape.current();
        } else if (e.key === "Tab") {
          e.preventDefault();
          cycle(el, e.shiftKey);
        }
      },
    };
    open.push(self);
    if (open.length === 1) window.addEventListener("keydown", onKeyDown);
    if (!keep && topModal() === self && !el.contains(back)) (initial?.current ?? el).focus({ preventScroll: true });

    return () => {
      open.splice(open.indexOf(self), 1);
      if (!open.length) window.removeEventListener("keydown", onKeyDown);
      if (keep) return;
      // Leave focus alone if closing moved it somewhere on purpose.
      const active = document.activeElement;
      if (active && active !== document.body && !el.contains(active)) return;
      // A dialog still open under this one takes it, unless focus came from there.
      const below = topModal()?.el;
      const target = below && !below.contains(back) ? below : back;
      if (target?.isConnected) target.focus({ preventScroll: true });
    };
  }, [ref, initial]);
}
