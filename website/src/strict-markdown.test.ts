import { describe, expect, test } from "vitest";
import { directivesUsed, fenceLanguage } from "./strict-markdown.ts";

const file = { path: "collie/runs.md" };
const at = (line: number) => ({ start: { line } });
const code = (lang: string | null, line: number) => ({ type: "code", lang, position: at(line) });

describe("fenceLanguage", () => {
  test("passes a page whose every fenced block names its language", () => {
    const tree = {
      type: "root",
      children: [code("ts", 3), { type: "list", children: [code("text", 9)] }],
    };

    expect(() => fenceLanguage()(tree, file)).not.toThrow();
  });

  test("fails on a fenced block with no language, naming its page and line", () => {
    const tree = { type: "root", children: [{ type: "blockquote", children: [code(null, 12)] }] };

    expect(() => fenceLanguage()(tree, file)).toThrow(
      "collie/runs.md:12: a fenced block names no language; use `text` for output and prose",
    );
  });
});

describe("directivesUsed", () => {
  test("passes the directives a plugin handled and a bare text directive", () => {
    const tree = {
      type: "root",
      children: [
        { type: "containerDirective", name: "note", data: { hName: "admonition" }, children: [] },
        { type: "paragraph", children: [{ type: "textDirective", name: "30", children: [] }] },
      ],
    };

    expect(() => directivesUsed()(tree, file)).not.toThrow();
  });

  test("fails on a directive no plugin handled, naming its page and line", () => {
    const tree = {
      type: "root",
      children: [{ type: "containerDirective", name: "nota", children: [], position: at(7) }],
    };

    expect(() => directivesUsed()(tree, file)).toThrow(
      "collie/runs.md:7: the directive `:::nota` is unused; no plugin renders it",
    );
  });
});
