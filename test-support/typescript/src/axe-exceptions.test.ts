import { describe, expect, test } from "vitest";
import { parseAxeExceptions } from "./axe-exceptions.ts";

const ENTRY = `[[exception]]
rule = "color-contrast"
selector = "#brand"
pages = ["/runs/:run_id"]
reason = "axe reads the gradient behind the text as its colour"
issue = 1701
`;

describe("parseAxeExceptions", () => {
  test("reads each entry's rule, selector, pages, reason and issue", () => {
    const exceptions = parseAxeExceptions(ENTRY);

    expect(exceptions).toEqual([
      {
        rule: "color-contrast",
        selector: "#brand",
        pages: ["/runs/:run_id"],
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
    ["pages", 'pages = ["/runs/:run_id"]\n'],
    ["reason", 'reason = "axe reads the gradient behind the text as its colour"\n'],
    ["issue", "issue = 1701\n"],
  ])("refuses an entry without its %s with axe_exceptions_invalid", ([, line]) => {
    const parsing = () => parseAxeExceptions(ENTRY.replace(line, ""));

    expect(parsing).toThrow(expect.objectContaining({ code: "axe_exceptions_invalid" }));
  });

  test.for<[string, string, string]>([
    ["rule", 'rule = "color-contrast"', 'rule = ""'],
    ["selector", 'selector = "#brand"', 'selector = "  "'],
    ["reason", 'reason = "axe reads the gradient behind the text as its colour"', 'reason = " "'],
  ])("refuses an entry whose %s is blank with axe_exceptions_invalid", ([, line, blank]) => {
    const parsing = () => parseAxeExceptions(ENTRY.replace(line, blank));

    expect(parsing).toThrow(expect.objectContaining({ code: "axe_exceptions_invalid" }));
  });

  test("refuses an entry naming no page with axe_exceptions_invalid", () => {
    const text = ENTRY.replace('pages = ["/runs/:run_id"]', "pages = []");

    const parsing = () => parseAxeExceptions(text);

    expect(parsing).toThrow(expect.objectContaining({ code: "axe_exceptions_invalid" }));
  });

  test("refuses a page that is no URL path pattern with axe_exceptions_invalid", () => {
    const text = ENTRY.replace('"/runs/:run_id"', '"/runs/(unclosed"');

    const parsing = () => parseAxeExceptions(text);

    expect(parsing).toThrow(expect.objectContaining({ code: "axe_exceptions_invalid" }));
  });

  test("refuses an entry with a field it does not define with axe_exceptions_invalid", () => {
    const text = `${ENTRY}disabled = true\n`;

    const parsing = () => parseAxeExceptions(text);

    expect(parsing).toThrow(expect.objectContaining({ code: "axe_exceptions_invalid" }));
  });
});
