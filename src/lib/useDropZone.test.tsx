// @vitest-environment happy-dom
// Files dropped from the computer are sent; a drag that starts inside the page
// never shows the drop overlay.

import { describe, expect, it, vi } from "vitest";
import { fire, render } from "../test/dom";
import { useDropZone } from "./useDropZone";

function Zone({ onFiles }: { onFiles: (f: File[]) => void }) {
  const drop = useDropZone(true, onFiles);
  return (
    <div id="zone" {...drop.handlers}>
      <span id="inner" draggable>
        text
      </span>
      {drop.dragging && <div id="overlay" />}
    </div>
  );
}

/** happy-dom's DragEvent leaves out dataTransfer, so it is set by hand: what a browser lists for a drag of files. */
function drag(type: string, files: File[] = []) {
  const e = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(e, "dataTransfer", { value: { types: files.length ? ["Files"] : ["text/plain"], files } });
  return e;
}

describe("useDropZone", () => {
  it("files from the computer show the overlay and are sent", () => {
    const onFiles = vi.fn();
    render(<Zone onFiles={onFiles} />);
    const zone = document.getElementById("zone")!;
    const file = new File(["x"], "IMG_0001.jpg");
    fire(zone, drag("dragover", [file]));
    expect(document.getElementById("overlay")).not.toBeNull();
    fire(zone, drag("drop", [file]));
    expect(document.getElementById("overlay")).toBeNull();
    expect(onFiles).toHaveBeenCalledWith([file]);
  });

  it("a drag from inside the page does not", () => {
    const onFiles = vi.fn();
    render(<Zone onFiles={onFiles} />);
    const zone = document.getElementById("zone")!;
    // iOS can list "Files" for a drag of selected text.
    fire(document.getElementById("inner"), drag("dragstart"));
    fire(zone, drag("dragover", [new File(["x"], "a.txt")]));
    expect(document.getElementById("overlay")).toBeNull();
    fire(zone, drag("drop", [new File(["x"], "a.txt")]));
    expect(onFiles).not.toHaveBeenCalled();
  });
});
