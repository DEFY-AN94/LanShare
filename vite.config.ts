import { readFileSync } from "node:fs";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

// iOS builds the Home Screen icon in a separate process that downloads the
// apple-touch-icon itself, and that download fails against our local
// certificate (the icon becomes a gray letter). A data: URL needs no download.
const TOUCH_ICON_LINK = 'rel="apple-touch-icon" sizes="180x180" href="/apple-touch-icon.png"';
const inlineTouchIcon: Plugin = {
  name: "inline-apple-touch-icon",
  transformIndexHtml(html) {
    if (!html.includes(TOUCH_ICON_LINK)) throw new Error("apple-touch-icon link not found in index.html");
    const png = readFileSync(new URL("./public/apple-touch-icon.png", import.meta.url)).toString("base64");
    return html.replace(TOUCH_ICON_LINK, `rel="apple-touch-icon" sizes="180x180" href="data:image/png;base64,${png}"`);
  },
};

// The built `dist/` is served by the Rust server to both the desktop window
// and phone browsers, so there is no Vite dev server in the loop.
export default defineConfig({
  plugins: [react(), inlineTouchIcon],
  clearScreen: false,
  build: {
    outDir: "dist",
    assetsDir: "assets",
    sourcemap: false,
  },
});
