import { defineConfig } from "vitest/config";

// Unit tests for the web UI's logic (no browser): `npm test`. Component tests
// (*.test.tsx) ask for a simulated page with `// @vitest-environment happy-dom`.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
