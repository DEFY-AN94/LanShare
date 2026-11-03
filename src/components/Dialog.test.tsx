// @vitest-environment happy-dom
// Confirm and prompt dialogs with the keyboard: focus moves in and back, Tab
// stays inside, Escape cancels, and a second request waits for the first.

import { describe, expect, it } from "vitest";
import { byText, click, flush, press, render } from "../test/dom";
import { DialogProvider, useDialog } from "./Dialog";

type Api = ReturnType<typeof useDialog>;
let api: Api;

function Page() {
  api = useDialog();
  return (
    <>
      <button>改名</button>
      <button>移除</button>
    </>
  );
}

function setup() {
  render(
    <DialogProvider>
      <Page />
    </DialogProvider>,
  );
  const trigger = byText("移除")!;
  trigger.focus();
  return trigger;
}

const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]');
const named = (attr: string) => document.getElementById(dialog()!.getAttribute(attr)!)?.textContent;

describe("confirm", () => {
  it("focuses 取消 for a delete, so Enter can't press the button behind it", async () => {
    const trigger = setup();
    let answer: boolean | undefined;
    void api.confirm({ title: "移除「Test iPad B」？", confirmText: "移除", danger: true }).then((v) => (answer = v));
    await flush();
    expect(document.activeElement).toBe(byText("取消"));
    expect(dialog()!.contains(document.activeElement)).toBe(true);
    click(byText("取消"));
    await flush();
    expect(answer).toBe(false);
    expect(dialog()).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("focuses the confirm button when nothing is deleted", async () => {
    setup();
    void api.confirm({ title: "登录时自动启动 LanShare？", confirmText: "自动启动" });
    await flush();
    expect(document.activeElement).toBe(byText("自动启动"));
  });

  it("is named by its title and described by its message", async () => {
    setup();
    void api.confirm({ title: "清空所有聊天记录？", message: "所有设备上的聊天记录都会删除，无法恢复。", danger: true });
    await flush();
    expect(named("aria-labelledby")).toBe("清空所有聊天记录？");
    expect(named("aria-describedby")).toBe("所有设备上的聊天记录都会删除，无法恢复。");
  });

  it("keeps Tab and Shift+Tab inside, also when focus has left it", async () => {
    setup();
    void api.confirm({ title: "删除这条消息？", confirmText: "删除", danger: true });
    await flush();
    const [cancel, ok] = [byText("取消")!, byText("删除")!];
    expect(press("Tab").defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(ok);
    press("Tab");
    expect(document.activeElement).toBe(cancel);
    press("Tab", { shiftKey: true });
    expect(document.activeElement).toBe(ok);
    (document.activeElement as HTMLElement).blur();
    press("Tab");
    expect(document.activeElement).toBe(cancel);
  });

  it("Escape answers false and gives focus back", async () => {
    const trigger = setup();
    let answer: boolean | undefined;
    void api.confirm({ title: "重置证书？", danger: true }).then((v) => (answer = v));
    await flush();
    press("Escape");
    await flush();
    expect(answer).toBe(false);
    expect(dialog()).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("a second request waits until the first is answered, then shows", async () => {
    const trigger = setup();
    const answers: unknown[] = [];
    void api.confirm({ title: "第一个", danger: true }).then((v) => answers.push(["first", v]));
    await flush();
    void api.confirm({ title: "第二个", confirmText: "好" }).then((v) => answers.push(["second", v]));
    await flush();
    expect(answers).toEqual([]);
    expect(document.querySelectorAll('[role="dialog"]')).toHaveLength(1);
    expect(dialog()!.textContent).toContain("第一个");
    press("Escape");
    await flush();
    expect(answers).toEqual([["first", false]]);
    expect(dialog()!.textContent).toContain("第二个");
    expect(document.activeElement).toBe(byText("好"));
    click(byText("好"));
    await flush();
    expect(answers).toEqual([
      ["first", false],
      ["second", true],
    ]);
    expect(dialog()).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });
});

describe("prompt", () => {
  it("focuses the input with the name before the extension selected", async () => {
    setup();
    void api.prompt({ title: "重命名", initial: "report.final.pdf", selectStem: true });
    await flush();
    const input = document.querySelector("input")!;
    expect(document.activeElement).toBe(input);
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, "report.final".length]);
  });

  it("Escape during IME composition does not close it", async () => {
    setup();
    let answer: string | null | undefined;
    void api.prompt({ title: "新建文件夹" }).then((v) => (answer = v));
    await flush();
    press("Escape", { isComposing: true });
    press("Escape", { keyCode: 229 });
    await flush();
    expect(dialog()).not.toBeNull();
    expect(answer).toBeUndefined();
    press("Escape");
    await flush();
    expect(answer).toBeNull();
  });

  it("a prompt that waited starts from its own text", async () => {
    setup();
    let first: string | null | undefined;
    let second: string | null | undefined;
    void api.prompt({ title: "修改设备名称", initial: "iPhone" }).then((v) => (first = v));
    await flush();
    void api.prompt({ title: "重命名", initial: "notes.txt" }).then((v) => (second = v));
    await flush();
    expect(document.querySelector("input")!.value).toBe("iPhone");
    click(byText("确定"));
    await flush();
    expect(first).toBe("iPhone");
    expect(document.querySelector("input")!.value).toBe("notes.txt");
    expect(document.activeElement).toBe(document.querySelector("input"));
    press("Escape");
    await flush();
    expect(second).toBeNull();
  });
});
