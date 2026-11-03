// @vitest-environment happy-dom
// A pairing request opens on its own, maybe while the user is typing: focus
// goes to the request itself, so a stray Enter or Escape decides nothing.

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PairRequest } from "../lib/types";
import { byText, click, flush, press, render } from "../test/dom";
import { PairingRequests } from "./PairingRequests";

const request: PairRequest = { id: "a1b2c3d4e5f6", name: "Test iPad B", confirm_code: "4821", ip: "192.168.1.20", created_at: 0 };

const api = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn() }));
vi.mock("../lib/api", () => ({ api }));
vi.mock("../lib/events", () => ({ events: { on: () => () => {} } }));

beforeEach(() => {
  api.get.mockReset().mockResolvedValue([request]);
  api.post.mockReset().mockResolvedValue({});
});

async function setup() {
  render(
    <>
      <textarea />
      <PairingRequests />
    </>,
  );
  const composer = document.querySelector("textarea")!;
  composer.focus();
  await flush();
  return composer;
}

const dialog = () => document.querySelector<HTMLElement>('[role="alertdialog"]');

describe("PairingRequests", () => {
  it("focus goes to the request itself, not to 允许 or 拒绝", async () => {
    await setup();
    expect(document.activeElement).toBe(dialog());
    expect(document.getElementById(dialog()!.getAttribute("aria-labelledby")!)?.textContent).toBe("「Test iPad B」请求连接");
    const described = dialog()!.getAttribute("aria-describedby")!.split(" ").map((id) => document.getElementById(id)?.textContent);
    expect(described).toContain("4821");
  });

  it("Enter and Escape neither decide nor close it", async () => {
    await setup();
    press("Enter");
    press("Escape");
    await flush();
    expect(dialog()).not.toBeNull();
    expect(api.post).not.toHaveBeenCalled();
  });

  it("after a decision focus goes back to where the user was typing", async () => {
    const composer = await setup();
    press("Tab");
    expect(document.activeElement).toBe(byText("拒绝"));
    click(byText("拒绝"));
    await flush();
    expect(api.post).toHaveBeenCalledWith("/api/host/pairing/requests/a1b2c3d4e5f6/reject");
    expect(dialog()).toBeNull();
    expect(document.activeElement).toBe(composer);
  });
});
