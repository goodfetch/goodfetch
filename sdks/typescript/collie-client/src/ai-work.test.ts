// AI work that can claim success (ADRs 0079 and 0117): how it ended, its
// Verdict and its model's claim are read apart, and `requireVerified()`
// hands over the output only when the work finished and its checks prove
// it, raising `not_verified` otherwise.
import { describe, expect, test } from "vitest";
import { AIWorkResult, type AIWorkFields } from "./ai-work.ts";
import { NotVerifiedError } from "./generated/index.ts";

interface Author {
  readonly name: string;
  readonly born: number;
}

const ADA: Author = { name: "Ada", born: 1815 };

function finished(fields: Partial<AIWorkFields<Author>> = {}): AIWorkResult<Author> {
  return new AIWorkResult<Author>({
    ended: { kind: "finished", output: ADA },
    verdict: "verified",
    verdictReasons: [],
    reported: { claim: "done", message: "found her" },
    ...fields,
  });
}

/** The `not_verified` that `result.requireVerified()` raises, or a failure when it returns. */
function refusal(result: AIWorkResult<Author>): NotVerifiedError {
  try {
    result.requireVerified();
  } catch (error) {
    if (error instanceof NotVerifiedError) {
      return error;
    }
    throw error;
  }
  throw new Error("the output was handed over");
}

describe("an AI work result", { tags: ["story:11-34", "error:not_verified"] }, () => {
  test("a verified finished result hands over its output", () => {
    expect(finished().requireVerified()).toEqual(ADA);
  });

  test("a finished result its checks did not prove raises not_verified with its Verdict, reasons and claim", () => {
    const error = refusal(
      finished({
        verdict: "rejected",
        verdictReasons: [
          { code: "success_check_failed", message: "the heading was not shown", callId: "c1" },
        ],
      }),
    );

    expect(error.code).toBe("not_verified");
    expect(error.group).toBe("page");
    expect(error.retryable).toBe(false);
    expect(error.details).toEqual({
      verdict: "rejected",
      reasons: [
        { code: "success_check_failed", message: "the heading was not shown", call_id: "c1" },
      ],
      reported: { claim: "done", message: "found her" },
    });
    expect(error.fix).toBeDefined();
    expect(error.message).toContain("rejected");
  });

  test("a stopped result raises not_verified even when its Verdict reads verified", () => {
    const error = refusal(
      finished({
        ended: { kind: "stopped", reason: { kind: "timeout" }, partial: { name: "Ada" } },
        reported: undefined,
      }),
    );

    expect(error.details).toEqual({ verdict: "verified", reasons: [] });
  });

  test("an unknown Verdict is never verified", () => {
    expect(refusal(finished({ verdict: "proven" })).details["verdict"]).toBe("proven");
  });

  test(
    "how it ended, its Verdict and the model's claim are read apart",
    { tags: ["story:11-35"] },
    () => {
      const result = finished({
        ended: { kind: "stopped", reason: { kind: "timeout" }, partial: { name: "Ada" } },
        verdict: "needs_review",
        verdictReasons: [{ code: "stopped", message: "the task stopped before it finished" }],
        reported: { claim: "done" },
      });

      const name =
        result.ended.kind === "finished" ? result.ended.output.name : result.ended.partial.name;

      expect(name).toBe("Ada");
      expect(result.verdict).toBe("needs_review");
      expect(result.reported?.claim).toBe("done");
    },
  );
});
