// @vitest-environment happy-dom
// A click on the backdrop closes the dialog; a press alone, a click inside, or
// a press inside that ends on the backdrop (selecting text) does not.

import { describe, expect, it, vi } from "vitest";
import { fire, render } from "../test/dom";
import { useBackdropClose } from "./useBackdropClose";

function Backdrop({ onClose }: { onClose: () => void }) {
  const backdrop = useBackdropClose(onClose);
  return (
    <div id="backdrop" {...backdrop}>
      <div id="dialog">notes.txt</div>
    </div>
  );
}

const down = (el: Element | null) => fire(el, new MouseEvent("mousedown", { bubbles: true }));
// A click goes to the element both the press and the release were on.
const up = (el: Element | null) => fire(el, new MouseEvent("click", { bubbles: true }));

function setup() {
  const onClose = vi.fn();
  render(<Backdrop onClose={onClose} />);
  return { onClose, backdrop: document.getElementById("backdrop"), dialog: document.getElementById("dialog") };
}

describe("useBackdropClose", () => {
  it("closes on the click, not on the press", () => {
    const { onClose, backdrop } = setup();
    down(backdrop);
    expect(onClose).not.toHaveBeenCalled();
    up(backdrop);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("a click inside the dialog does not close it", () => {
    const { onClose, dialog } = setup();
    down(dialog);
    up(dialog);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("a press inside that ends on the backdrop does not close it", () => {
    const { onClose, backdrop, dialog } = setup();
    down(dialog);
    up(backdrop);
    expect(onClose).not.toHaveBeenCalled();
    // The next click on the backdrop starts afresh.
    down(backdrop);
    up(backdrop);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
