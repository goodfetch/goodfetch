// The kit's sample test under `node:test`, run by `node --test` alone.
import { test } from "node:test";
import { aRunFinishesOnTheTestEngine } from "./sample.ts";

await test("a Run finishes on the test engine", aRunFinishesOnTheTestEngine);
