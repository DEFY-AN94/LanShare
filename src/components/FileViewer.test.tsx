// @vitest-environment happy-dom
// In the Home Screen app the save sheet opens over the viewer: Escape closes
// the sheet first, then the viewer, and focus goes back one step each time.

import { beforeEach, describe, expect, it, vi } from "vitest";
import { byText, click, press, render } from "../test/dom";
import { useFileOpener } from "./FileViewer";

function Files() {
  const opener = useFileOpener();
  return (
    <>
      <button onClick={() => opener.preview({ url: "/api/files/download?path=IMG_0001.jpg", name: "IMG_0001.jpg", size: 1000 })}>IMG_0001.jpg</button>
      {opener.element}
    </>
  );
}

beforeEach(() => {
  // The Home Screen app on an iPhone that can share files.
  vi.spyOn(window, "matchMedia").mockImplementation((query: string) => ({ matches: query === "(display-mode: standalone)" || query === "(pointer: coarse)", media: query }) as MediaQueryList);
  Object.assign(navigator, { share: () => Promise.resolve(), canShare: () => true });
  // The file is still being read when the sheet is closed.
  vi.spyOn(window, "fetch").mockReturnValue(new Promise(() => {}));
});

describe("FileViewer", () => {
  it("Escape closes only the save sheet over it, then the viewer", () => {
    render(<Files />);
    const file = byText("IMG_0001.jpg")!;
    file.focus();
    click(file);
    const viewer = document.querySelector<HTMLElement>(".viewer")!;
    expect(document.activeElement).toBe(viewer);
    const save = byText("下载到「文件」")!;
    save.focus();
    click(save);
    expect(document.querySelector(".modal")!.contains(document.activeElement)).toBe(true);
    press("Escape");
    expect(document.querySelector(".modal")).toBeNull();
    expect(document.querySelector(".viewer")).not.toBeNull();
    expect(document.activeElement).toBe(save);
    press("Escape");
    expect(document.querySelector(".viewer")).toBeNull();
    expect(document.activeElement).toBe(file);
  });
});
