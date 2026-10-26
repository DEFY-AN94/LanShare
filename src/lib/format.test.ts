// Which parts of a chat message become links.

import { expect, it } from "vitest";
import { decodeText, fingerprintRows, splitLinks, viewKind } from "./format";

it("keeps trailing punctuation as text beside the link", () => {
  const punctuation = ".,;:!?)]}'\"";
  expect(splitLinks(`See https://example.com/path${punctuation} next.`)).toEqual([
    { text: "See " },
    { text: "https://example.com/path", href: "https://example.com/path" },
    { text: `${punctuation} next.` },
  ]);
});

it("preserves the complete text and hrefs across multiple URLs", () => {
  const text = "链接：https://example.com/one), and HTTP://example.org/two?q=3。 done";
  const parts = splitLinks(text);
  expect(parts).toEqual([
    { text: "链接：" },
    { text: "https://example.com/one", href: "https://example.com/one" },
    { text: "), and " },
    { text: "HTTP://example.org/two?q=3", href: "HTTP://example.org/two?q=3" },
    { text: "。 done" },
  ]);
  expect(parts.map((part) => part.text).join("")).toBe(text);
});

it("only links HTTP and HTTPS protocols", () => {
  const plain = "ftp://example.com file:///tmp/x javascript:alert(1) data:text/plain,x mailto:a@example.com";
  expect(splitLinks(plain)).toEqual([{ text: plain }]);
  expect(splitLinks("http://example.com HTTPS://example.org")).toEqual([
    { text: "http://example.com", href: "http://example.com" },
    { text: " " },
    { text: "HTTPS://example.org", href: "HTTPS://example.org" },
  ]);
  expect(splitLinks("")).toEqual([]);
});

it("handles a long punctuation suffix without losing text", () => {
  const punctuation = ".,;:!?)]}".repeat(20_000);
  const url = "https://example.com/path";
  expect(splitLinks(url + punctuation)).toEqual([
    { text: url, href: url },
    { text: punctuation },
  ]);
});

it("handles a long punctuation run ending in a nonmatch without retrying the run", () => {
  const url = `https://example.com/${"!".repeat(200_000)}x`;
  expect(splitLinks(url)).toEqual([{ text: url, href: url }]);
});

it("shows a SHA-256 fingerprint as iPhone does, 8 pairs to a row", () => {
  const hex = "0123456789abcdef".repeat(4);
  expect(fingerprintRows(hex)).toEqual(Array(4).fill("01 23 45 67 89 AB CD EF"));
  expect(fingerprintRows("")).toEqual([]);
});

it("shows images, video, audio and text in the Home Screen viewer, and saves the rest", () => {
  expect(viewKind("photo.HEIC")).toBe("image");
  expect(viewKind("clip.mov")).toBe("video");
  expect(viewKind("voice.m4a")).toBe("audio");
  expect(viewKind("notes.md")).toBe("text");
  // PDFs cannot be framed in the page, so they go to the share sheet.
  for (const name of ["doc.pdf", "x.svg", "scan.tiff", "a.zip", "README"]) expect(viewKind(name)).toBeNull();
});

it("decodes UTF-8, UTF-16 and GB18030 text", () => {
  const utf8 = new TextEncoder().encode("中文 text");
  expect(decodeText(utf8)).toBe("中文 text");
  expect(decodeText(new Uint8Array([0xef, 0xbb, 0xbf, ...utf8]))).toBe("中文 text");
  expect(decodeText(new Uint8Array([0xff, 0xfe, 0x2d, 0x4e, 0x87, 0x65]))).toBe("中文");
  expect(decodeText(new Uint8Array([0xfe, 0xff, 0x4e, 0x2d, 0x65, 0x87]))).toBe("中文");
  // 中文 as saved by Chinese Windows: not valid UTF-8.
  expect(decodeText(new Uint8Array([0xd6, 0xd0, 0xce, 0xc4]))).toBe("中文");
});

it("drops a character split where a long text file is cut off", () => {
  const bytes = new TextEncoder().encode("ab中");
  expect(decodeText(bytes.subarray(0, 4), true)).toBe("ab");
  // The same bytes as a whole file are not UTF-8, so they are read as GB18030.
  expect(decodeText(bytes.subarray(0, 4))).not.toBe("ab");
});
