// Which parts of a chat message become links.

import { expect, it } from "vitest";
import { fingerprintRows, splitLinks } from "./format";

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
