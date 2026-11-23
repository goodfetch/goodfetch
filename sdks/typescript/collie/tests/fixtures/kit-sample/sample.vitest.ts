// The kit's sample test under Vitest, with `vitest.config.ts` beside it.
import { test } from "vitest";
import { aRunFinishesOnTheTestEngine } from "./sample.ts";

// oxlint-disable-next-line vitest/expect-expect -- the sample asserts with `node:assert`, in `sample.ts`.
test("a Run finishes on the test engine", aRunFinishesOnTheTestEngine);
