import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    retry: 0,
    sequence: { shuffle: { tests: true } },
  },
});
