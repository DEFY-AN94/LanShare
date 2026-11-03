// @vitest-environment happy-dom
// What opens a text message's actions: a tap on the iPhone; with a mouse, the
// ⋯ button or a right-click, so that a click or double-click only selects.

import { describe, expect, it, vi } from "vitest";
import { byLabel, click, fire, pointer, render } from "../test/dom";
import type { Message } from "../lib/types";
import { MessageBubble } from "./MessageBubble";

const text = (id: number, body: string) => ({ id, kind: "text", text: body, from_host: false, created_at: 0 }) as Message;
const message = text(7, "hello wonderful world https://example.com");

const bubble = (m: Message, onActions: (m: Message) => void, isHost: boolean) => (
  <MessageBubble message={m} mine={false} isHost={isHost} onMediaLoad={() => {}} onShare={() => {}} onOpenFile={() => {}} onActions={onActions} />
);

function setup(kind: "coarse" | "fine") {
  pointer(kind);
  const onActions = vi.fn();
  render(bubble(message, onActions, kind === "fine"));
  const el = document.querySelector<HTMLElement>(".bubble")!;
  const word = Array.from(el.querySelectorAll("span")).find((s) => s.textContent?.includes("wonderful"))!;
  return { onActions, word, link: el.querySelector("a")! };
}

const select = (node: Node) => window.getSelection()!.selectAllChildren(node);
const pressRight = (el: Element) => fire(el, new PointerEvent("pointerdown", { bubbles: true, button: 2 }));
const menu = (el: Element) => fire(el, new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 }));
const rightClick = (el: Element) => {
  pressRight(el);
  return menu(el);
};

describe("with a mouse (the computer)", () => {
  it("a click only selects", () => {
    const { onActions, word } = setup("fine");
    click(word);
    fire(word, new MouseEvent("click", { bubbles: true, detail: 2 }));
    expect(onActions).not.toHaveBeenCalled();
  });

  it("the ⋯ button opens the actions", () => {
    const { onActions } = setup("fine");
    const more = byLabel("更多操作")!;
    expect(more.tagName).toBe("BUTTON");
    click(more);
    expect(onActions).toHaveBeenCalledWith(message);
  });

  it("a right-click opens them instead of the system menu", () => {
    const { onActions, word } = setup("fine");
    expect(rightClick(word).defaultPrevented).toBe(true);
    expect(onActions).toHaveBeenCalledTimes(1);
  });

  it("a Mac selects the word under a right-click after the press: that is not a selection to keep", () => {
    const { onActions, word } = setup("fine");
    pressRight(word);
    select(word);
    expect(menu(word).defaultPrevented).toBe(true);
    expect(onActions).toHaveBeenCalledTimes(1);
    expect(window.getSelection()!.toString()).toBe("");
  });

  it("a right-click on a selection or a link keeps the system menu", () => {
    const { onActions, word, link } = setup("fine");
    select(word);
    expect(rightClick(word).defaultPrevented).toBe(false);
    window.getSelection()!.removeAllRanges();
    expect(rightClick(link).defaultPrevented).toBe(false);
    expect(onActions).not.toHaveBeenCalled();
  });

  it("text selected in another message does not change what a right-click on this one does", () => {
    pointer("fine");
    const onActions = vi.fn();
    const second = text(8, "second message");
    render(
      <>
        {bubble(text(7, "first message"), onActions, true)}
        {bubble(second, onActions, true)}
      </>,
    );
    const [first, other] = Array.from(document.querySelectorAll(".bubble"));
    select(first);
    expect(rightClick(other.firstElementChild!).defaultPrevented).toBe(true);
    expect(onActions.mock.calls).toEqual([[second]]);
  });
});

describe("on a touch screen (the iPhone)", () => {
  it("a tap opens the actions, but not one on a link or one that ends a selection", () => {
    const { onActions, word, link } = setup("coarse");
    link.addEventListener("click", (e) => e.preventDefault());
    click(link);
    select(word);
    click(word);
    expect(onActions).not.toHaveBeenCalled();
    window.getSelection()!.removeAllRanges();
    click(word);
    expect(onActions).toHaveBeenCalledTimes(1);
  });

  it("a long press is left to iOS", () => {
    const { onActions, word } = setup("coarse");
    expect(rightClick(word).defaultPrevented).toBe(false);
    expect(onActions).not.toHaveBeenCalled();
  });
});
