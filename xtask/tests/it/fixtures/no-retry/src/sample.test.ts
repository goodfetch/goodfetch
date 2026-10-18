import { expect, test, vi } from "vitest";

test("reads a value once it is written", async () => {
  let value = 0;
  setTimeout(() => {
    value = 1;
  }, 10);
  // Waiting on a condition with a deadline is not a retry (ADR 0064).
  await vi.waitFor(() => expect(value).toBe(1), { timeout: 1_000 });
});
