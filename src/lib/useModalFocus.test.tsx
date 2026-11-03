// @vitest-environment happy-dom
// Focus with two dialogs open at once (a pairing request over 添加设备, the save
// sheet over the viewer), and on the iPhone with the keyboard up.

import { act, useRef, useState, type ReactNode } from "react";
import { flushSync } from "react-dom";
import { describe, expect, it, vi } from "vitest";
import { byText, pointer, press, render } from "../test/dom";
import { useModalFocus } from "./useModalFocus";

function Modal({ name, onEscape, children }: { name: string; onEscape?: () => void; children?: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useModalFocus(ref, { onEscape });
  return (
    <div ref={ref} role="dialog" aria-modal="true" aria-label={name} tabIndex={-1}>
      <button>{`${name} 取消`}</button>
      {children}
      <button>{`${name} 确定`}</button>
    </div>
  );
}

function Page({ lower, upper, onEscape }: { lower: boolean; upper: boolean; onEscape?: (name: string) => void }) {
  return (
    <>
      <textarea />
      <button>添加设备</button>
      {lower && <Modal name="A" onEscape={() => onEscape?.("A")} />}
      {upper && <Modal name="B" onEscape={() => onEscape?.("B")} />}
    </>
  );
}

const dialog = (name: string) => document.querySelector(`[aria-label="${name}"]`);

describe("useModalFocus", () => {
  it("only the dialog on top answers keys, and closing it hands focus back into the one below", () => {
    const onEscape = vi.fn();
    const page = render(<Page lower={false} upper={false} onEscape={onEscape} />);
    byText("添加设备")!.focus();
    page.rerender(<Page lower upper={false} onEscape={onEscape} />);
    expect(document.activeElement).toBe(dialog("A"));
    byText("A 确定")!.focus();
    page.rerender(<Page lower upper onEscape={onEscape} />);
    expect(document.activeElement).toBe(dialog("B"));
    press("Tab");
    press("Tab");
    press("Tab");
    expect(dialog("B")!.contains(document.activeElement)).toBe(true);
    press("Escape");
    expect(onEscape.mock.calls).toEqual([["B"]]);
    page.rerender(<Page lower upper={false} onEscape={onEscape} />);
    expect(document.activeElement).toBe(byText("A 确定"));
    page.rerender(<Page lower={false} upper={false} onEscape={onEscape} />);
    expect(document.activeElement).toBe(byText("添加设备"));
  });

  it("a dialog that opens under another one waits for it", () => {
    const page = render(<Page lower={false} upper />);
    expect(document.activeElement).toBe(dialog("B"));
    page.rerender(<Page lower upper />);
    expect(document.activeElement).toBe(dialog("B"));
    page.rerender(<Page lower upper={false} />);
    expect(document.activeElement).toBe(dialog("A"));
  });

  it("one Escape closes only the dialog on top, also when the one below opened later", () => {
    const onEscapeA = vi.fn();
    let openA = () => {};
    function Stack() {
      const [a, setA] = useState(false);
      const [b, setB] = useState(true);
      openA = () => setA(true);
      return (
        <>
          {a && <Modal name="A" onEscape={onEscapeA} />}
          {/* A real key press lets React commit between two window listeners; flushSync does that here. */}
          {b && <Modal name="B" onEscape={() => flushSync(() => setB(false))} />}
        </>
      );
    }
    render(<Stack />);
    act(() => openA());
    expect(dialog("A")).not.toBeNull();
    press("Escape");
    expect(dialog("B")).toBeNull();
    expect(onEscapeA).not.toHaveBeenCalled();
    expect(dialog("A")!.contains(document.activeElement)).toBe(true);
  });

  it("leaves focus where closing put it on purpose", () => {
    function Rename() {
      const [open, setOpen] = useState(false);
      const field = useRef<HTMLInputElement>(null);
      return (
        <>
          <button onClick={() => setOpen(true)}>改名</button>
          <input ref={field} aria-label="名称" />
          {open && (
            <Modal
              name="A"
              onEscape={() => {
                field.current!.focus();
                setOpen(false);
              }}
            />
          )}
        </>
      );
    }
    render(<Rename />);
    const button = byText("改名")!;
    button.focus();
    act(() => button.click());
    expect(document.activeElement).toBe(dialog("A"));
    press("Escape");
    expect(dialog("A")).toBeNull();
    expect(document.activeElement).toBe(document.querySelector('[aria-label="名称"]'));
  });

  it("Tab also reaches the controls of a video or audio player", () => {
    render(
      <Modal name="viewer">
        <video controls />
      </Modal>,
    );
    press("Tab");
    press("Tab");
    expect(document.activeElement?.tagName).toBe("VIDEO");
  });

  it("on the iPhone a text field keeps focus, so the keyboard stays as it was", () => {
    pointer("coarse");
    const page = render(<Page lower={false} upper={false} />);
    const input = document.querySelector("textarea")!;
    input.focus();
    page.rerender(<Page lower upper={false} />);
    expect(document.activeElement).toBe(input);
    page.rerender(<Page lower={false} upper={false} />);
    expect(document.activeElement).toBe(input);
  });
});
