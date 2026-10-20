// Client conformance cases as an SDK's tests carry them (ADR 0069), run by
// the case-log reporter's tests: one passes, one fails a check, one is
// skipped, and one test carries no case.
import { expect } from "vitest";
import { test } from "@goodfetch/test-support";

test(
  "refuses a newer major",
  { tags: ["client_case/hello-refused", "attached"] },
  async ({ clientCase }) => {
    clientCase.message({ method: "hello", params: { protocol: "1.4" } });
    clientCase.message({ error: { code: "protocol_unsupported" } });

    await clientCase.check("the Client raises protocol_unsupported", () => {
      expect("protocol_unsupported").toBe("protocol_unsupported");
    });
  },
);

test(
  "cancels a run mid-way",
  { tags: ["client_case/cancel-mid-run", "attached"] },
  async ({ clientCase }) => {
    await clientCase.check("the call sends $/cancel", () => {
      expect("$/cancel").toBe("$/cancel");
    });
    await clientCase.check("the run ends cancelled", () => {
      expect("completed").toBe("cancelled");
    });
  },
);

test.skip("releases its credits", { tags: ["client_case/credits-released", "daemon"] }, () => {});

test("carries no case", () => {
  expect(true).toBe(true);
});
