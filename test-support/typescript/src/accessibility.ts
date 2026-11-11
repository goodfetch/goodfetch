// axe's audit of a served test's page (ADR 0073): WCAG 2.2 AA and axe's best
// practices, every rule on.
// The `axe` fixture audits every open page at the end of each served test;
// `expectAccessible(page)` audits a state on the way (an open dialog, drawer
// or menu). "Incomplete" results are attached to the report and never fail.
import { AxeBuilder } from "@axe-core/playwright";
import { test, type Page, type TestInfo } from "@playwright/test";
import { TestSupportError } from "./errors.ts";

/** The axe tags every audit runs: WCAG 2.0, 2.1 and 2.2 at A and AA, and best practices. */
export const AXE_TAGS: readonly string[] = [
  "wcag2a",
  "wcag2aa",
  "wcag21a",
  "wcag21aa",
  "wcag22aa",
  "best-practice",
];

/** Where an audit reports. */
export interface AuditOptions {
  /** The test whose report receives the incomplete results. */
  readonly testInfo: TestInfo;
}

type AxeResults = Awaited<ReturnType<AxeBuilder["analyze"]>>;
type AxeResult = AxeResults["violations"][number];

/**
 * Audits `page` with axe. Fails with `axe_violations` when the page has a
 * violation. Attaches the page's incomplete results to the test as
 * `axe-incomplete`.
 */
export async function auditAccessibility(page: Page, options: AuditOptions): Promise<void> {
  const results = await new AxeBuilder({ page }).withTags([...AXE_TAGS]).analyze();
  const violations = results.violations;

  if (results.incomplete.length > 0) {
    await options.testInfo.attach("axe-incomplete", {
      body: JSON.stringify(
        { url: page.url(), incomplete: results.incomplete.map(summary) },
        null,
        2,
      ),
      contentType: "application/json",
    });
  }
  if (violations.length > 0) {
    throw new TestSupportError(
      "axe_violations",
      `axe found ${violations.length} violation(s) on ${page.url()}:\n${violations.map(described).join("\n")}`,
    );
  }
}

/** Audits `page` as the `axe` fixture does at the end of a test. */
export async function expectAccessible(page: Page): Promise<void> {
  await auditAccessibility(page, {
    testInfo: test.info(),
  });
}

function summary(result: AxeResult): object {
  return {
    id: result.id,
    impact: result.impact,
    help: result.help,
    helpUrl: result.helpUrl,
    nodes: result.nodes.map(({ target, failureSummary }) => ({ target, failureSummary })),
  };
}

function described(result: AxeResult): string {
  const nodes = result.nodes.map(({ target }) => `    ${JSON.stringify(target)}`).join("\n");
  return `- ${result.id} (${result.impact ?? "no impact"}): ${result.help} ${result.helpUrl}\n${nodes}`;
}
