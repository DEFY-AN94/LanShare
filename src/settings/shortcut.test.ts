import { describe, expect, it } from "vitest";
import { acceleratorFromEvent, displayAccelerator, windowsCode } from "./shortcut";

const key = (code: string, mods: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {}) => ({
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe("recording the clipboard shortcut", () => {
  it("records a key with two or more modifiers, in a fixed order", () => {
    expect(acceleratorFromEvent(key("KeyV", { metaKey: true, ctrlKey: true, altKey: true }))).toBe("Control+Alt+Super+KeyV");
    expect(acceleratorFromEvent(key("Digit5", { ctrlKey: true, shiftKey: true }))).toBe("Control+Shift+Digit5");
    expect(acceleratorFromEvent(key("F9", { altKey: true, metaKey: true }))).toBe("Alt+Super+F9");
  });

  it("ignores combinations that type a character", () => {
    expect(acceleratorFromEvent(key("Digit7", { altKey: true, shiftKey: true })), "⌥⇧7 types a symbol").toBeNull();
    expect(acceleratorFromEvent(key("KeyV", { ctrlKey: true, altKey: true }), true), "Ctrl+Alt is AltGr on Windows").toBeNull();
    expect(acceleratorFromEvent(key("KeyE", { ctrlKey: true, altKey: true, shiftKey: true }), true)).toBeNull();
    expect(acceleratorFromEvent(key("KeyV", { ctrlKey: true, altKey: true })), "fine on a Mac").toBe("Control+Alt+KeyV");
    expect(acceleratorFromEvent(key("KeyV", { ctrlKey: true, shiftKey: true }), true)).toBe("Control+Shift+KeyV");
  });

  it("ignores plain typing, single-modifier slips and keys the system cannot register", () => {
    expect(acceleratorFromEvent(key("KeyV"))).toBeNull();
    expect(acceleratorFromEvent(key("KeyV", { shiftKey: true }))).toBeNull();
    expect(acceleratorFromEvent(key("KeyV", { metaKey: true })), "⌘V would take over pasting").toBeNull();
    expect(acceleratorFromEvent(key("KeyC", { ctrlKey: true })), "Ctrl+C would take over copying").toBeNull();
    expect(acceleratorFromEvent(key("ControlLeft", { ctrlKey: true, altKey: true })), "modifiers on their own").toBeNull();
    expect(acceleratorFromEvent(key("KeyV", { altKey: true, shiftKey: true })), "needs Control or Command").toBeNull();
    expect(acceleratorFromEvent(key("MediaPlayPause", { ctrlKey: true, altKey: true }))).toBeNull();
  });

  it("names the key by the virtual key Windows reports, whatever the layout", () => {
    expect(windowsCode(90), "German: the key in the Y position is Z").toBe("KeyZ");
    expect(windowsCode(48)).toBe("Digit0");
    expect(windowsCode(120)).toBe("F9");
    expect(windowsCode(32)).toBe("Space");
    expect(windowsCode(222), "a punctuation key: it moves between layouts").toBeNull();
    expect(windowsCode(192), "UK: the ' key").toBeNull();
    expect(windowsCode(229), "an input method composing").toBeNull();
  });

  it("shows it the way each system writes shortcuts", () => {
    expect(displayAccelerator("Control+Alt+Super+KeyV", "macos")).toBe("⌃⌥⌘V");
    expect(displayAccelerator("Control+Shift+Digit5", "windows")).toBe("Ctrl+Shift+5");
    expect(displayAccelerator("Alt+Space", "macos")).toBe("⌥空格");
  });
});
