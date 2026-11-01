// Every form of the Console renders `Form` (ADR 0073), so Enter during an
// input method's composition submits none of them; `tests/screens/
// composition.test.tsx` shows what `Form` does in the Runs filter and a
// setting's edit. A bare `<form>` anywhere else in Collie's Console would
// submit on that Enter.
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { expect, test } from "vitest";

/** The Console's component files, its tests, stories and `Form` itself aside. */
function components(folder: string): string[] {
  return readdirSync(folder, { recursive: true, encoding: "utf8" })
    .filter((path) => path.endsWith(".tsx"))
    .filter((path) => !/\.(test|stories)\.tsx$/u.test(path) && path !== "form.tsx")
    .map((path) => join(folder, path));
}

test("no Console component renders a bare <form>, each renders Form", () => {
  const folder = import.meta.dirname;
  const bare = components(folder)
    .filter((path) => /<form[\s>]/u.test(readFileSync(path, "utf8")))
    .map((path) => relative(folder, path));
  expect(bare).toEqual([]);
});
