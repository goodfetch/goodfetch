// A snapshot whose stored value differs: CI must fail it and write nothing.
import { expect, test } from "vitest";

test("greeting", () => {
  expect("hello").toMatchSnapshot();
});
