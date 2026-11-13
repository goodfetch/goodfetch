// Foxhound's Connection on the Scripted engine (ADR 0069): each test plays a
// script of the shared corpus over the in-memory transport, as an Attached
// engine's stdio would carry it. The script fails the test on a message it
// does not expect, `hello` included, and on a step left unused.
import { playScript, test } from "@goodfetch/test-support";
import { Collie } from "@goodfetch/collie";
import { connectOver as connectOverCollie } from "@goodfetch/collie/testing";
import * as client from "@goodfetch/collie-client";
import { describe, expect } from "vitest";
import { processHandlers } from "../tests/process-handlers.ts";
import { startAndFinish } from "../tests/programs/tracer.ts";
import { Foxhound } from "./foxhound.ts";
import * as foxhoundPackage from "./index.ts";
import { connectOver } from "./testing.ts";

/** The error `promise` rejects with. */
async function failure(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("the call succeeded");
}

/** The error `Foxhound.connect()` fails with when `env` is its environment map. */
function connectFailure(env: Readonly<Record<string, string>>): Promise<unknown> {
  // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
  return failure(Foxhound.connect({ env }));
}

/** The members a class's values and the class itself carry, its constructor left out. */
function members(type: { readonly prototype: object }): string[] {
  return [
    ...Reflect.ownKeys(type.prototype).filter((key) => key !== "constructor"),
    ...Reflect.ownKeys(type).filter(
      (key) => !["length", "name", "prototype"].includes(String(key)),
    ),
  ]
    .map(String)
    .toSorted();
}

describe("a Foxhound Connection", () => {
  test(
    "fails connect() when a Collie engine answers, with the engine's details and fix, before any other call",
    { tags: ["story:11-1005", "error:not_in_product"] },
    async () => {
      const played = playScript("connect/foxhound-refused-by-collie.json");

      const error = await failure(connectOver(played.transport));

      expect(error).toBeInstanceOf(client.NotInProductError);
      expect(error).toMatchObject({
        code: "not_in_product",
        group: "unavailable",
        retryable: false,
        fix: "Connect to a Foxhound engine, or run with Foxhound, which contains everything Collie does (`npm install @goodfetch/foxhound`).",
        details: {
          needs: "foxhound",
          product: "collie",
          product_version: "0.0.0",
          install: "npm install @goodfetch/foxhound",
        },
      });
      expect(played.done()).toEqual({ kind: "closed" });
    },
  );

  test(
    "has finished hello, naming Foxhound, once connect() returns",
    { tags: ["story:11-1002"] },
    async () => {
      const played = playScript("connect/foxhound-accepted.json");

      const foxhound = await connectOver(played.transport);
      const engine = foxhound.engine();
      await foxhound.close();

      expect(engine).toEqual({
        kind: "attached",
        product: "foxhound",
        version: "0.0.0",
        protocolVersion: "0.1",
        enginePath: null,
      });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "opens many times in one process, at once and after a close, and installs no handler in it",
    { tags: ["story:11-1002"] },
    async () => {
      const before = processHandlers();
      const scripts = ["first", "second"].map(() =>
        playScript("run/foxhound-started-and-finished.json"),
      );

      const traced = await Promise.all(
        scripts.map(async (played) => startAndFinish(await connectOver(played.transport))),
      );
      const again = playScript("connect/foxhound-accepted.json");
      await (await connectOver(again.transport)).close();
      // The factory itself, as far as starting its engine's process.
      const unstarted = await connectFailure({
        GOODFETCH_FOXHOUND_ENGINE_PATH: "/nonexistent/goodfetch-foxhound",
      });

      expect({
        lifecycles: traced.map(({ report }) => report.lifecycle),
        played: [...scripts, again].map((played) => played.done().kind),
        unstarted: unstarted instanceof client.EngineNotFoundError,
        handlers: processHandlers(),
      }).toEqual({
        lifecycles: ["finished", "finished"],
        played: ["finished", "finished", "finished"],
        unstarted: true,
        handlers: before,
      });
    },
  );

  test(
    "carries every member of a Collie Connection under the same name",
    { tags: ["story:11-1003"] },
    () => {
      expect(members(Foxhound)).toEqual(expect.arrayContaining(members(Collie)));
    },
  );

  test(
    "is taken by a helper typed for Connection as a Collie Connection is, and a Collie error class catches either one's error",
    { tags: ["story:11-1003", "story:11-1004"] },
    async () => {
      const onFoxhound = playScript("run/foxhound-started-and-finished.json");
      const onCollie = playScript("run/started-and-finished.json");
      const connections: readonly client.Connection[] = [
        await connectOver(onFoxhound.transport),
        await connectOverCollie(onCollie.transport),
      ];

      const traced = [];
      const closed = [];
      for (const connection of connections) {
        traced.push(await startAndFinish(connection));
        closed.push((await failure(connection.start())) instanceof client.ClosedError);
      }

      const finished = {
        startedAs: "new",
        runId: "01ke6p4ym0j8x3v5wq2rtbn7hc",
        report: { lifecycle: "finished", verdict: "verified", completeness: "complete" },
      };
      expect(traced).toMatchObject([finished, finished]);
      expect(closed).toEqual([true, true]);
      expect([onFoxhound.done(), onCollie.done()]).toEqual([
        { kind: "finished" },
        { kind: "finished" },
      ]);
    },
  );

  test(
    "hands out the client package's own values and error classes, which its package re-exports",
    { tags: ["story:11-1004"] },
    async () => {
      const played = playScript("run/foxhound-started-and-finished.json");
      const foxhound = await connectOver(played.transport);
      const run = await foxhound.start();
      await run.finish();
      await foxhound.close();

      const closed = await failure(foxhound.start());

      expect({
        run: run instanceof client.Run,
        closed: closed instanceof client.ClosedError,
        reexported: [
          foxhoundPackage.Run === client.Run,
          foxhoundPackage.ClosedError === client.ClosedError,
          foxhoundPackage.NotInProductError === client.NotInProductError,
        ],
      }).toEqual({ run: true, closed: true, reexported: [true, true, true] });
    },
  );
});

describe("Foxhound.connect()", () => {
  test(
    "reads only Foxhound's engine-path variable, never Collie's",
    { tags: ["story:11-1007"] },
    async () => {
      const error = await connectFailure({ GOODFETCH_COLLIE_ENGINE_PATH: process.execPath });

      const platformPackage = `@goodfetch/foxhound-${process.platform}-${process.arch}`;
      expect(error).toMatchObject({
        code: "engine_not_found",
        details: { package: platformPackage },
        fix: `Install @goodfetch/foxhound with its optional dependencies, which hold its engine in ${platformPackage}, or set GOODFETCH_FOXHOUND_ENGINE_PATH to Foxhound's executable.`,
      });
    },
  );

  test(
    "starts the executable its engine-path variable names, and names the variable when it is none",
    { tags: ["story:11-1007", "error:engine_not_found"] },
    async () => {
      const path = "/nonexistent/goodfetch-foxhound";

      const error = await connectFailure({ GOODFETCH_FOXHOUND_ENGINE_PATH: path });

      expect(error).toMatchObject({
        code: "engine_not_found",
        group: "unavailable",
        details: { path },
        fix: `GOODFETCH_FOXHOUND_ENGINE_PATH names ${path}, which is no executable file: point it at Foxhound's executable, goodfetch-foxhound, or leave it unset to use the one @goodfetch/foxhound installed.`,
      });
    },
  );

  test(
    "fails when its engine-path variable is set but empty, naming the variable",
    { tags: ["story:11-1007", "error:setting_invalid"] },
    async () => {
      const error = await connectFailure({ GOODFETCH_FOXHOUND_ENGINE_PATH: "" });

      expect(error).toMatchObject({
        code: "setting_invalid",
        group: "program",
        details: { key: "GOODFETCH_FOXHOUND_ENGINE_PATH" },
      });
    },
  );
});
