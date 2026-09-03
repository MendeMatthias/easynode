import { defineConfig } from "vitest/config";

// The node app's TS is a thin Tauri veneer; only pure logic (the ask free-text
// router) gets unit tests. Runs headless in Node.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
