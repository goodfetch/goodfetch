// axe's audit of a served test's page (ADR 0073): WCAG 2.2 AA and axe's best
// practices, every rule on, with the repository's exception file applied.
// The `axe` fixture audits every open page at the end of each served test;
// `expectAccessible(page)` audits a state on the way (an open dialog, drawer
// or menu). "Incomplete" results are attached to the report and never fail.
import { AxeBuilder } from "@axe-core/playwright";
import { test, type Page, type TestInfo } from "@playwright/test";
import { namesPage, readAxeExceptions, type AxeException } from "./axe-exceptions.ts";
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

/** What an audit applies, and where it reports. */
export interface AuditOptions {
  /** The known false positives; the fixture and `expectAccessible` pass the repository's file. */
  readonly exceptions: readonly AxeException[];
  /** The test whose report receives the incomplete results. */
  readonly testInfo: TestInfo;
}

type AxeResults = Awaited<ReturnType<AxeBuilder["analyze"]>>;
type AxeResult = AxeResults["violations"][number];
type AxeNode = AxeResult["nodes"][number];

/**
 * Audits `page` with axe. Fails with `axe_exception_too_broad` when an
 * exception that names the page has a selector taking in the whole page
 * (`html`, `body` or every element in it), which would turn its rule off;
 * with `axe_violations` when a violation is left once each exception that
 * names the page has suppressed its rule on the elements its selector
 * matches; or with `axe_exception_unmatched` when an exception names the
 * page, its rule ran there and it excepted no violation there, its selector
 * matching nothing or no element of it violating the rule any more. Attaches
 * the page's incomplete results to the test as `axe-incomplete`.
 */
export async function auditAccessibility(page: Page, options: AuditOptions): Promise<void> {
  const results = await new AxeBuilder({ page }).withTags([...AXE_TAGS]).analyze();
  const ran = new Set(
    [...results.passes, ...results.violations, ...results.incomplete].map(({ id }) => id),
  );
  const applied = options.exceptions.filter(
    (exception) => ran.has(exception.rule) && namesPage(exception, page.url()),
  );
  const unmatched: AxeException[] = [];
  const tooBroad: AxeException[] = [];
  let violations = results.violations;
  for (const exception of applied) {
    const matched = await matchedNodes(page, exception, violations);
    if (matched.wholePage) {
      tooBroad.push(exception);
      continue;
    }
    if (matched.nodes.size === 0) {
      unmatched.push(exception);
    }
    violations = violations
      .map((violation) =>
        violation.id === exception.rule
          ? { ...violation, nodes: violation.nodes.filter((node) => !matched.nodes.has(node)) }
          : violation,
      )
      .filter(({ nodes }) => nodes.length > 0);
  }

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
  if (tooBroad.length > 0) {
    throw new TestSupportError(
      "axe_exception_too_broad",
      `on ${page.url()}, these axe exceptions' selectors take in the whole page, which would turn their rule off; name the misjudged elements only:\n${tooBroad
        .map(({ rule, selector, issue }) => `- ${rule} on \`${selector}\` (#${issue})`)
        .join("\n")}`,
    );
  }
  if (violations.length > 0) {
    throw new TestSupportError(
      "axe_violations",
      `axe found ${violations.length} violation(s) on ${page.url()}:\n${violations.map(described).join("\n")}`,
    );
  }
  if (unmatched.length > 0) {
    throw new TestSupportError(
      "axe_exception_unmatched",
      `on ${page.url()}, where their rule ran, these axe exceptions excepted no violation: their selector matches nothing, or none of its elements violates the rule any more; fix or remove them:\n${unmatched
        .map(({ rule, selector, issue }) => `- ${rule} on \`${selector}\` (#${issue})`)
        .join("\n")}`,
    );
  }
}

/** Audits `page` as the `axe` fixture does at the end of a test, with the repository's exception file. */
export async function expectAccessible(page: Page): Promise<void> {
  await auditAccessibility(page, {
    exceptions: await readAxeExceptions(),
    testInfo: test.info(),
  });
}

/**
 * Which of its rule's violating nodes `exception`'s selector matches, and
 * whether it takes in the whole page: `html`, `body` or every element in
 * it. A node inside a frame or a shadow root, whose target is a path of
 * selectors, is never excepted.
 */
async function matchedNodes(
  page: Page,
  exception: AxeException,
  violations: readonly AxeResult[],
): Promise<{ wholePage: boolean; nodes: ReadonlySet<AxeNode> }> {
  const nodes = violations
    .filter(({ id }) => id === exception.rule)
    .flatMap(({ nodes: ruleNodes }) => ruleNodes);
  const targets = nodes.map(({ target }) =>
    target.length === 1 && typeof target[0] === "string" ? target[0] : null,
  );
  const { wholePage, matches } = await page.evaluate(
    ({ selector, targets: nodeTargets }) => {
      try {
        const inBody = document.body.querySelectorAll("*").length;
        return {
          wholePage:
            document.documentElement.matches(selector) ||
            document.body.matches(selector) ||
            (inBody > 0 && document.body.querySelectorAll(selector).length === inBody),
          matches: nodeTargets.map(
            (target) =>
              target !== null && document.querySelector(target)?.matches(selector) === true,
          ),
        };
      } catch {
        // An invalid selector matches nothing.
        return { wholePage: false, matches: nodeTargets.map(() => false) };
      }
    },
    { selector: exception.selector, targets },
  );
  return { wholePage, nodes: new Set(nodes.filter((_, index) => matches[index] === true)) };
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
