// Closing a dialog or sheet with a tap on the backdrop around it. It closes on
// the click, not the press: closing on the press let the rest of the tap land
// on whatever was under the backdrop (on iPhone, focusing an input there and
// raising the keyboard). A press that starts inside the dialog and ends on the
// backdrop, such as selecting text, does not close it either.

import { useRef, type MouseEvent } from "react";

export function useBackdropClose(onClose: () => void) {
  const pressed = useRef(false);
  return {
    onMouseDown: (e: MouseEvent) => {
      pressed.current = e.target === e.currentTarget;
    },
    onClick: (e: MouseEvent) => {
      if (pressed.current && e.target === e.currentTarget) onClose();
      pressed.current = false;
    },
  };
}
