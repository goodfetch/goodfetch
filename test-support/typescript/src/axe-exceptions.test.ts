import { describe, expect, test } from "vitest";
import { parseAxeExceptions } from "./axe-exceptions.ts";

const ENTRY = `[[exception]]
rule = "color-contrast"
selector = "#brand"
reason = "axe reads the gradient behind the text as its colour"
issue = 1701
`;

describe("parseAxeExceptions", () => {
  test("reads each entry's rule, selector, reason and issue", () => {
    const exceptions = parseAxeExceptions(ENTRY);

    expect(exceptions).toEqual([
      {
        rule: "color-contrast",
        selector: "#brand",
        reason: "axe reads the gradient behind the text as its colour",
        issue: 1701,
      },
    ]);
  });

  test("reads a file of comments only as no entry", () => {
    const exceptions = parseAxeExceptions("# No false positive yet.\n");

    expect(exceptions).toEqual([]);
  });

  test.for<[string, string]>([
    ["rule", 'rule = "color-contrast"\n'],
    ["selector", 'selector = "#brand"\n'],
    ["reason", 'reason = "axe reads the gradient behind the text as its colour"\n'],
    ["issue", "issue = 1701\n"],
  ])("refuses an entry without its %s with axe_exceptions_invalid", ([, line]) => {
    const parsing = () => parseAxeExceptions(ENTRY.replace(line, ""));

    expect(parsing).toThrow(expect.objectContaining({ code: "axe_exceptions_invalid" }));
  });

  test("refuses an entry with a field it does not define with axe_exceptions_invalid", () => {
    const text = `${ENTRY}disabled = true\n`;

    const parsing = () => parseAxeExceptions(text);

    expect(parsing).toThrow(expect.objectContaining({ code: "axe_exceptions_invalid" }));
  });
});
