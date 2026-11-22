// Preloaded by each Node program a test starts while StrykerJS learns which
// tests cover each mutant (`mutation-children.ts`). StrykerJS's instrumented
// code counts each mutant it covers in `globalThis.__stryker__`, which this
// creates first: each mutant id counted the first time is appended at once
// to the file `STRYKER_CHILD_COVERAGE` names, so a program killed by a
// signal has still written its own.
import { appendFileSync } from "node:fs";

const file = process.env["STRYKER_CHILD_COVERAGE"];
if (file !== undefined) {
  const covered = new Proxy<Record<string, number>>(
    {},
    {
      set(counts, id, count: number) {
        if (typeof id === "string" && !(id in counts)) {
          appendFileSync(file, `${id}\n`);
        }
        return Reflect.set(counts, id, count);
      },
    },
  );
  // oxlint-disable-next-line no-underscore-dangle -- StrykerJS's own name for its namespace.
  globalThis.__stryker__ = { mutantCoverage: { static: covered, perTest: {} } };
}
