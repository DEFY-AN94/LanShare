// Dropping files from the computer onto the chat or the file list. A drag that
// starts inside the page (text selected in the file viewer, an image in the
// chat) is not a drop of files: iOS can still list "Files" for it and end it
// without a drop or leave event, which left "松开即可上传" on screen. Such a
// drag never shows the overlay and is left to the browser.

import { useRef, useState, type DragEvent } from "react";

export function useDropZone(enabled: boolean, onFiles: (files: File[]) => void) {
  const [dragging, setDragging] = useState(false);
  const own = useRef(false);

  const handlers = {
    onDragStart: () => {
      own.current = true;
    },
    onDragEnd: () => {
      own.current = false;
      setDragging(false);
    },
    onDragOver: (e: DragEvent) => {
      if (own.current || !enabled || !e.dataTransfer.types.includes("Files")) return;
      e.preventDefault();
      setDragging(true);
    },
    onDragLeave: (e: DragEvent) => {
      if (e.currentTarget.contains(e.relatedTarget as Node)) return;
      setDragging(false);
    },
    onDrop: (e: DragEvent) => {
      setDragging(false);
      if (own.current) {
        own.current = false;
        return;
      }
      e.preventDefault();
      const files = Array.from(e.dataTransfer.files);
      if (files.length && enabled) onFiles(files);
    },
  };
  return { dragging, handlers };
}
