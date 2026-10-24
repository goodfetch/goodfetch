// `routes.json`, which the Feature inventory and Collie's executable read,
// lists exactly the routes the build declares (ADR 0072), and a route's
// `$` segments stand for any one segment of the page's path, as the
// executable serves them.
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test } from "vitest";
import { ROUTES, RUN, RUNS, paramsOf } from "./routes.ts";

test("routes.json lists every route of Collie's build, and no other", async () => {
  const committed: unknown = JSON.parse(
    await readFile(join(import.meta.dirname, "..", "routes.json"), "utf8"),
  );

  expect(committed).toEqual({ routes: ROUTES });
});

test.each([
  { route: RUNS, path: "/runs", params: {} },
  { route: RUN, path: "/runs/nightly-run", params: { run: "nightly-run" } },
  { route: RUN, path: "/runs/a%20run", params: { run: "a run" } },
  { route: RUN, path: "/runs", params: undefined },
  { route: RUN, path: "/runs/", params: undefined },
  { route: RUN, path: "/runs/nightly-run/more", params: undefined },
  { route: RUNS, path: "/runs/nightly-run", params: undefined },
])("$path is $route's with $params", ({ route, path, params }) => {
  expect(paramsOf(route, path)).toEqual(params);
});
