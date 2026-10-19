import { describe, expect, test } from "vitest";

describe("echo", { tags: ["operation:sample.echo"] }, () => {
  test("repeats the text", () => {
    expect(1).toBe(1);
  });
});

test("declares no Feature", () => {
  expect(1).toBe(1);
});
