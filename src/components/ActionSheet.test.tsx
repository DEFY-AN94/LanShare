// @vitest-environment happy-dom
// The actions sheet with the keyboard, and handing focus on to the confirm
// that one of its actions opens.

import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { byText, click, flush, press, render } from "../test/dom";
import { ActionSheet, type SheetAction } from "./ActionSheet";
import { DialogProvider, useDialog } from "./Dialog";

type Confirm = ReturnType<typeof useDialog>["confirm"];

function Page({ title, actions }: { title?: string; actions: (confirm: Confirm) => SheetAction[] }) {
  const [open, setOpen] = useState(false);
  const dialog = useDialog();
  return (
    <>
      <button aria-label="notes.txt 的更多操作" onClick={() => setOpen(true)}>
        ⋯
      </button>
      {open && <ActionSheet title={title} actions={actions(dialog.confirm)} onClose={() => setOpen(false)} />}
    </>
  );
}

function setup(actions: (confirm: Confirm) => SheetAction[], title: string | null = "notes.txt") {
  render(
    <DialogProvider>
      <Page title={title ?? undefined} actions={actions} />
    </DialogProvider>,
  );
  const trigger = document.querySelector<HTMLElement>('[aria-label="notes.txt 的更多操作"]')!;
  trigger.focus();
  click(trigger);
  return trigger;
}

const sheet = () => document.querySelector<HTMLElement>(".sheet");

describe("ActionSheet", () => {
  it("is a dialog named by its title, with focus on the first action", () => {
    setup(() => [
      { label: "重命名", onSelect: () => {} },
      { label: "删除", danger: true, onSelect: () => {} },
    ]);
    expect(sheet()!.getAttribute("role")).toBe("dialog");
    expect(sheet()!.getAttribute("aria-modal")).toBe("true");
    expect(document.querySelector('[role="menu"], [role="menuitem"]')).toBeNull();
    expect(document.getElementById(sheet()!.getAttribute("aria-labelledby")!)?.textContent).toBe("notes.txt");
    expect(document.activeElement).toBe(byText("重命名"));
  });

  it("without a title it is still named", () => {
    setup(() => [{ label: "复制", onSelect: () => {} }], null);
    expect(sheet()!.getAttribute("aria-label")).toBe("更多操作");
    expect(document.activeElement).toBe(byText("复制"));
  });

  it("starts on 取消 when every action deletes", () => {
    setup(() => [{ label: "删除消息，保留文件", danger: true, onSelect: () => {} }]);
    expect(document.activeElement).toBe(byText("取消"));
  });

  it("Escape closes it and focus goes back to its button", () => {
    const trigger = setup(() => [{ label: "重命名", onSelect: () => {} }]);
    press("Escape");
    expect(sheet()).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("an action that asks to confirm hands focus to the confirm, then back", async () => {
    const remove = vi.fn();
    const trigger = setup((confirm) => [
      { label: "删除", danger: true, onSelect: () => void confirm({ title: "删除「notes.txt」？", confirmText: "删除", danger: true }).then(remove) },
    ]);
    click(byText("删除"));
    await flush();
    expect(sheet()).toBeNull();
    expect(document.activeElement).toBe(byText("取消"));
    expect(document.querySelector('[role="dialog"]')!.contains(document.activeElement)).toBe(true);
    press("Escape");
    await flush();
    expect(remove).toHaveBeenCalledWith(false);
    expect(document.activeElement).toBe(trigger);
  });
});
