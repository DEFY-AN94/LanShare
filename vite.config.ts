import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The built `dist/` is served by the Rust server to both the desktop window
// and phone browsers, so there is no Vite dev server in the loop.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  build: {
    outDir: "dist",
    assetsDir: "assets",
    sourcemap: false,
  },
});
