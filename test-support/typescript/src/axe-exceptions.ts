// The axe exception file, `console/axe-exceptions.toml` (ADR 0073): axe's
// known false positives, one `[[exception]]` entry each, naming the rule,
// the selector of the elements it is excepted on, the pages it is excepted
// on, the reason and its open issue. A test never disables a rule; the axe fixture reads this file.
// Whether each issue is still open is `xtask axe-exceptions check`'s job, as
// a test cannot reach GitHub (ADR 0043).
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { parse } from "smol-toml";
import { TestSupportError } from "./errors.ts";

/** One known false positive of axe. */
export interface AxeException {
  /** The axe rule's ID, such as `color-contrast`. */
  readonly rule: string;
  /** A CSS selector of the elements the rule is excepted on. */
  readonly selector: string;
  /**
   * The pages it is excepted on, each a URL path pattern as `URLPattern`
   * reads one (`/runs/:run_id`; `/*` for every page), so an entry is scoped
   * to the screens where axe is wrong.
   */
  readonly pages: readonly string[];
  /** Why axe is wrong there. */
  readonly reason: string;
  /** The number of its open issue, which the fix closes. */
  readonly issue: number;
}

/** The repository's exception file. */
export const AXE_EXCEPTIONS_FILE: string = join(
  import.meta.dirname,
  "..",
  "..",
  "..",
  "console",
  "axe-exceptions.toml",
);

const FIELDS: readonly string[] = ["rule", "selector", "pages", "reason", "issue"];

/** The entries of an exception file's text; fails with `axe_exceptions_invalid` on an entry missing a field, or with one it does not define. */
export function parseAxeExceptions(text: string): readonly AxeException[] {
  let document: Record<string, unknown>;
  try {
    document = parse(text);
  } catch (error) {
    throw new TestSupportError("axe_exceptions_invalid", "the axe exception file is no TOML", {
      cause: error,
    });
  }
  const unknownKeys = Object.keys(document).filter((key) => key !== "exception");
  if (unknownKeys.length > 0) {
    throw new TestSupportError(
      "axe_exceptions_invalid",
      `the axe exception file has keys it does not define: ${unknownKeys.join(", ")}`,
    );
  }
  const entries = document["exception"] ?? [];
  if (!Array.isArray(entries)) {
    throw new TestSupportError(
      "axe_exceptions_invalid",
      "the axe exception file's `exception` is no array of tables",
    );
  }
  return entries.map((entry: unknown, index) => exceptionAt(entry, index + 1));
}

function exceptionAt(entry: unknown, position: number): AxeException {
  const invalid = (why: string): TestSupportError =>
    new TestSupportError("axe_exceptions_invalid", `axe exception ${position} ${why}`);
  if (typeof entry !== "object" || entry === null || Array.isArray(entry)) {
    throw invalid("is no table");
  }
  const fields: Record<string, unknown> = { ...entry };
  const unknownFields = Object.keys(fields).filter((field) => !FIELDS.includes(field));
  if (unknownFields.length > 0) {
    throw invalid(`has fields it does not define: ${unknownFields.join(", ")}`);
  }
  const { rule, selector, pages, reason, issue } = fields;
  if (typeof rule !== "string" || rule.trim() === "") {
    throw invalid("has no rule");
  }
  if (typeof selector !== "string" || selector.trim() === "") {
    throw invalid("has no selector");
  }
  if (!Array.isArray(pages) || pages.length === 0) {
    throw invalid("names no page");
  }
  const patterns: string[] = [];
  for (const pattern of pages) {
    if (typeof pattern !== "string" || !isPathPattern(pattern)) {
      throw invalid(`has a page that is no URL path pattern: ${JSON.stringify(pattern)}`);
    }
    patterns.push(pattern);
  }
  if (typeof reason !== "string" || reason.trim() === "") {
    throw invalid("has no reason");
  }
  if (typeof issue !== "number" || !Number.isSafeInteger(issue) || issue < 1) {
    throw invalid("has no issue number");
  }
  return { rule, selector, pages: patterns, reason, issue };
}

/** Whether `pattern` is a URL path pattern `URLPattern` reads. */
function isPathPattern(pattern: string): boolean {
  try {
    return new URLPattern({ pathname: pattern }) instanceof URLPattern;
  } catch {
    return false;
  }
}

/** Whether `exception` names the page at `url`, by its path. */
export function namesPage(exception: AxeException, url: string): boolean {
  return exception.pages.some((pathname) =>
    new URLPattern({ pathname }).test({ pathname: new URL(url).pathname }),
  );
}

let committed: Promise<readonly AxeException[]> | undefined;

/** The entries of the repository's exception file, read once per worker. */
export function readAxeExceptions(): Promise<readonly AxeException[]> {
  committed ??= readFile(AXE_EXCEPTIONS_FILE, "utf8").then(parseAxeExceptions, (error: unknown) => {
    throw new TestSupportError(
      "axe_exceptions_invalid",
      `the axe exception file ${AXE_EXCEPTIONS_FILE} cannot be read`,
      { cause: error },
    );
  });
  return committed;
}
