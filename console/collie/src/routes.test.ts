// `routes.json`, which the Feature inventory and Collie's executable read,
// lists exactly the routes the build declares (ADR 0072).
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test } from "vitest";
import { ROUTES } from "./routes.ts";

test("routes.json lists every route of Collie's build, and no other", async () => {
  const committed: unknown = JSON.parse(
    await readFile(join(import.meta.dirname, "..", "routes.json"), "utf8"),
  );

  expect(committed).toEqual({ routes: ROUTES });
});
