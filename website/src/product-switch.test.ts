import { describe, expect, test } from "vitest";
import { switchTarget } from "./product-switch.ts";

const docs = [
  { id: "index", path: "/docs/next/" },
  { id: "collie/index", path: "/docs/next/collie" },
  { id: "collie/install", path: "/docs/next/collie/install" },
  { id: "collie/runs", path: "/docs/next/collie/runs" },
  { id: "foxhound/index", path: "/docs/next/foxhound" },
  { id: "foxhound/runs", path: "/docs/next/foxhound/runs" },
];

describe("switchTarget", () => {
  test("keeps the page when the other section has it", () => {
    expect(switchTarget(docs, "collie/runs", "foxhound")).toBe("/docs/next/foxhound/runs");
  });

  test("goes to the other section's home when it lacks the page", () => {
    expect(switchTarget(docs, "collie/install", "foxhound")).toBe("/docs/next/foxhound");
  });

  test("goes to a section's home from a root page or outside the docs", () => {
    expect(switchTarget(docs, "index", "collie")).toBe("/docs/next/collie");
    expect(switchTarget(docs, undefined, "foxhound")).toBe("/docs/next/foxhound");
  });
});
