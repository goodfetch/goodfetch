// A passing snapshot beside one no test references: CI must fail the run.
import { expect, test } from "vitest";

test("greeting", () => {
  expect("hello").toMatchSnapshot();
});
