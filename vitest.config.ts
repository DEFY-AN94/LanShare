import { defineConfig } from "vitest/config";

// Unit tests for the web UI's logic (no browser): `npm test`.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
