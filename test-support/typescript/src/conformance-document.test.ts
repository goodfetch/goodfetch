import { describe, expect, test } from "vitest";
import { conformanceDocument, type ConformanceFacts } from "./conformance-document.ts";
import { conformanceVectors } from "./outcome-vectors.ts";

const header = {
  suite: { name: "console", version: "1" },
  target: { name: "collie-daemon", product: "collie", version: "0.0.0" },
  platform: { os: "linux", arch: "x86_64" },
  runtime: "chromium-141",
  proving_ground: { version: "0.0.0+abc", seed: 1 },
  online: false,
} as const;

/** One capability whose one case ran once and whose one check held. */
const proved: ConformanceFacts = {
  ...header,
  capabilities: [
    {
      name: "runs.list",
      cases: [
        {
          id: "runs-list-lists-every-run",
          covers: [],
          entries: [{ checks: [{ name: "the test passed", passed: true }] }],
        },
      ],
    },
  ],
};

describe("conformanceDocument", () => {
  test.for(conformanceVectors(header))(
    "gives the outcomes of the vector $name",
    ({ facts, caseOutcomes, outcome }) => {
      const [capability] = conformanceDocument(facts).capabilities;

      expect(capability?.cases.map((one) => one.outcome)).toEqual(caseOutcomes);
      expect(capability?.outcome).toBe(outcome);
    },
  );

  test("writes the conformance kind", () => {
    expect(conformanceDocument(proved).kind).toBe("conformance");
  });

  test("derives the outcomes over ones the facts carry", () => {
    const capability = { name: "runs.list", cases: [], outcome: "proved" };
    const smuggled: ConformanceFacts = { ...header, capabilities: [capability] };

    expect(conformanceDocument(smuggled).capabilities[0]?.outcome).toBe("unproved");
  });
});
