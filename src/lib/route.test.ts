// @vitest-environment happy-dom
// Folder names survive the trip through the address's #: %, #, ?, / and Chinese.

import { afterEach, describe, expect, it } from "vitest";
import { navigate, parseHash, replaceRoute } from "./route";

afterEach(() => history.replaceState(null, "", "/"));

describe("route", () => {
  it.each([[["files", "照片", "2025"]], [["files", "100% done", "a#b?c"]], [["files", "a/b"]], [["chat", "a1b2c3d4e5f6"]]])("%j round-trips", (segments) => {
    navigate(...segments);
    expect(parseHash()).toEqual(segments);
    replaceRoute("files");
    replaceRoute(...segments);
    expect(parseHash()).toEqual(segments);
  });

  it("drops empty segments and keeps a malformed one as typed", () => {
    navigate("files", "", "x");
    expect(parseHash()).toEqual(["files", "x"]);
    history.replaceState(null, "", "#/files/%E0%A4%A");
    expect(parseHash()).toEqual(["files", "%E0%A4%A"]);
  });
});
