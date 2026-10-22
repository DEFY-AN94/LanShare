// The global shortcut that sends the clipboard, as Settings records and
// shows it. The stored form is a Tauri accelerator ("Control+Alt+Super+KeyV"),
// which the shell registers with tauri-plugin-global-shortcut.

export interface KeyLike {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/** Keys the shortcut may end with (what the plugin can register). */
const KEY = /^(Key[A-Z]|Digit[0-9]|F([1-9]|1[0-2])|Backquote|Minus|Equal|BracketLeft|BracketRight|Backslash|Semicolon|Quote|Comma|Period|Slash|Space)$/;

/**
 * The accelerator for a key press, or null while it is not a usable
 * shortcut (the server checks the same rules):
 * - at least two modifiers, so a slip such as ⌘V or Ctrl+C never takes over
 *   pasting or copying in every app;
 * - one of them Control or ⌘ / Win, and on Windows not Ctrl+Alt without
 *   Win: Option(+Shift) types symbols on a Mac, and Ctrl+Alt is AltGr on
 *   Windows, so those would take a character away from typing.
 */
export function acceleratorFromEvent(e: KeyLike, windows = false): string | null {
  const modifiers = [e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter((m): m is string => !!m);
  if (!KEY.test(e.code) || modifiers.length < 2) return null;
  if (!e.ctrlKey && !e.metaKey) return null;
  if (windows && e.ctrlKey && e.altKey && !e.metaKey) return null;
  return [...modifiers, e.code].join("+");
}

/**
 * On Windows the plugin registers a shortcut by virtual key, and which
 * physical key has which virtual key depends on the keyboard layout (on a
 * German keyboard the key in the Y position is Z). WebView2 reports the
 * virtual key as `keyCode`, so the key is named from that: letters, digits,
 * F1–F12 and Space. Punctuation keys are refused, since their virtual keys
 * move between layouts (UK, Japanese, Turkish…) and the label would not
 * match the key.
 */
export function windowsCode(keyCode: number): string | null {
  if (keyCode >= 65 && keyCode <= 90) return `Key${String.fromCharCode(keyCode)}`;
  if (keyCode >= 48 && keyCode <= 57) return `Digit${keyCode - 48}`;
  if (keyCode >= 112 && keyCode <= 123) return `F${keyCode - 111}`;
  return keyCode === 32 ? "Space" : null;
}

const MAC_SYMBOLS: Record<string, string> = { Control: "⌃", Alt: "⌥", Shift: "⇧", Super: "⌘" };
const WIN_NAMES: Record<string, string> = { Control: "Ctrl", Alt: "Alt", Shift: "Shift", Super: "Win" };
const KEY_NAMES: Record<string, string> = {
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Space: "空格",
};

function keyName(code: string): string {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  return KEY_NAMES[code] ?? code;
}

/** "⌃⌥⌘V" on a Mac, "Ctrl+Alt+Win+V" on Windows. */
export function displayAccelerator(accelerator: string, platform: string): string {
  const parts = accelerator.split("+");
  const key = keyName(parts.pop() ?? "");
  if (platform === "macos") return parts.map((p) => MAC_SYMBOLS[p] ?? p).join("") + key;
  return [...parts.map((p) => WIN_NAMES[p] ?? p), key].join("+");
}
