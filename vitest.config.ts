import { defineConfig } from "vitest/config";

// Unit tests for the web UI's logic (no browser): `npm test`. A test that needs
// a page (the component tests, *.test.tsx) asks for a simulated one in its first
// line: `// @vitest-environment happy-dom`.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
