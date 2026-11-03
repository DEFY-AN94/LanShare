// @vitest-environment happy-dom
// Enter sends on the computer, but never while an IME (pinyin) is composing,
// and never on the iPhone, whose return key makes a new line.

import { act } from "react";
import { describe, expect, it, vi } from "vitest";
import { pointer, press, render } from "../test/dom";
import { Composer } from "./Composer";

let n = 0;

function setup(kind: "coarse" | "fine") {
  pointer(kind);
  const onSendText = vi.fn();
  // A draft key of its own each time: drafts are kept per conversation.
  render(<Composer draftKey={`test-${n++}`} onSendText={onSendText} onSendFiles={() => {}} />);
  const input = document.querySelector("textarea")!;
  input.focus();
  // As typing does: the value changes, then an input event.
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "你好");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  return { onSendText, input };
}

describe("Composer", () => {
  it("Enter sends", () => {
    const { onSendText, input } = setup("fine");
    expect(press("Enter").defaultPrevented).toBe(true);
    expect(onSendText).toHaveBeenCalledWith("你好");
    expect(input.value).toBe("");
  });

  it("Enter that ends an IME composition does not send", () => {
    const { onSendText, input } = setup("fine");
    expect(press("Enter", { isComposing: true }).defaultPrevented).toBe(false);
    // Safari reports the composition only through keyCode 229.
    expect(press("Enter", { keyCode: 229 }).defaultPrevented).toBe(false);
    expect(onSendText).not.toHaveBeenCalled();
    expect(input.value).toBe("你好");
  });

  it("Shift+Enter makes a new line", () => {
    const { onSendText } = setup("fine");
    expect(press("Enter", { shiftKey: true }).defaultPrevented).toBe(false);
    expect(onSendText).not.toHaveBeenCalled();
  });

  it("on the iPhone Enter makes a new line", () => {
    const { onSendText } = setup("coarse");
    expect(press("Enter").defaultPrevented).toBe(false);
    expect(onSendText).not.toHaveBeenCalled();
  });
});
