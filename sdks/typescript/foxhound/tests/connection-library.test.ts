// A library serving both Products (ADR 0095): `connection-library`, a package
// of this workspace, takes the Collie client package as a peer dependency, so
// a program's Connection meets the library's own import of the client package
// as the program's one copy. Each Product's Connection plays a script of the
// shared corpus over the in-memory transport (ADR 0069).
import { connectOver as connectOverCollie } from "@goodfetch/collie/testing";
import { connectOver } from "@goodfetch/foxhound/testing";
import { playScript, test } from "@goodfetch/test-support";
import { finishOneRun } from "connection-library";
import { describe, expect } from "vitest";

const products = [
  {
    product: "Foxhound",
    open: connectOver,
    finished: "run/foxhound-started-and-finished.json",
    closed: "connect/foxhound-accepted.json",
  },
  {
    product: "Collie",
    open: connectOverCollie,
    finished: "run/started-and-finished.json",
    closed: "connect/reached-attached.json",
  },
] as const;

describe("connection-library's finishOneRun", () => {
  test.for(products)(
    "takes $product's Connection and finishes a Run on it",
    { tags: ["story:11-180"] },
    async ({ open, finished }) => {
      const played = playScript(finished);
      const connection = await open(played.transport);

      const report = await finishOneRun(connection);

      expect(report).toMatchObject({ lifecycle: "finished" });
      await connection.close();
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test.for(products)(
    "catches, with its own import of the client package, the error $product's closed Connection fails with",
    { tags: ["story:11-180"] },
    async ({ open, closed }) => {
      const played = playScript(closed);
      const connection = await open(played.transport);
      await connection.close();

      const answer = await finishOneRun(connection);

      expect(answer).toBe("closed");
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );
});
