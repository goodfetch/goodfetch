// The Playwright `test` of the Console conformance suite (ADRs 0068 and
// 0096): test support's served `test`, whose every test also hands the
// reporter what its target is, so the target's `conformance` document can
// say what ran against what.
//
// A test runs the case of the capability its `capability` annotation names
// whose ID is its title, such as a test titled `runs-list-lists-every-run`
// annotated `{ type: "capability", description: "runs.list" }`. The project names its target as
// `metadata.conformanceTarget` and sets the `product` whose Daemon is the
// target and a `provingGroundSeed`: the document names the Proving ground
// the cases ran against, so a test whose Daemon has none fails with
// `proving_ground_unseeded`. Before the body, the test attaches its
// target's Product and version (as the Daemon's executable reports them
// through `--version`), this host's platform, the browser's runtime and the
// Proving ground's version and seed; `ResultDocumentReporter` writes the
// target's document from them.
import type {
  PlaywrightTestArgs,
  PlaywrightTestOptions,
  PlaywrightWorkerArgs,
  PlaywrightWorkerOptions,
  TestType,
} from "@playwright/test";
import { browserRuntime, hostPlatform } from "./case-log.ts";
import { daemonCommand, reportedVersion } from "./daemon-command.ts";
import { TestSupportError } from "./errors.ts";
import { test as served, type ServedFixtures, type ServedWorkerFixtures } from "./playwright.ts";
import { ProcessRecord } from "./process-record.ts";
import { CONFORMANCE_FACTS, type TargetFacts } from "./result-document-reporter.ts";

export { expect } from "@playwright/test";
export { CAPABILITY_ANNOTATION, CONFORMANCE_TARGET } from "./result-document-reporter.ts";

/** What a Console conformance test takes beside a served test's fixtures. */
export interface ConformanceFixtures {
  /** Attaches the test's target's facts before its body; every test takes it. */
  conformanceTarget: void;
}

/** Test support's served `test`, whose every test reports its Console conformance target. */
export const test: TestType<
  PlaywrightTestArgs & PlaywrightTestOptions & ServedFixtures & ConformanceFixtures,
  PlaywrightWorkerArgs & PlaywrightWorkerOptions & ServedWorkerFixtures
> = served.extend<ConformanceFixtures>({
  conformanceTarget: [
    async ({ daemon, browser, browserName }, use, testInfo) => {
      const provingGround = daemon.provingGround;
      if (provingGround === undefined) {
        throw new TestSupportError(
          "proving_ground_unseeded",
          "a Console conformance test runs against a Proving ground: set `provingGroundSeed` in its project",
        );
      }
      const processes = new ProcessRecord();
      try {
        const facts: TargetFacts = {
          target: await reportedVersion(
            (args) => daemonCommand(processes, daemon, args),
            daemon.executable,
          ),
          platform: hostPlatform(),
          runtime: browserRuntime(browserName, browser.version()),
          proving_ground: { version: provingGround.version, seed: provingGround.seed },
          online: false,
        };
        await testInfo.attach(CONFORMANCE_FACTS, {
          body: JSON.stringify(facts),
          contentType: "application/json",
        });
      } finally {
        await processes.close();
      }
      await use();
    },
    { auto: true },
  ],
});
