// A snapshot with no stored value: CI must fail it and write nothing.
import { expect, test } from "vitest";

test("greeting", () => {
  expect("hello").toMatchSnapshot();
});
