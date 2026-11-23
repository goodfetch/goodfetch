// A program's `run.checkpoint()` takes no path: a path on the engine's
// machine would break on a Daemon or Goodfetch Cloud, so only the
// catalog's caller gives `location`, while `run.migrate()` keeps its own
// (ADR 0102).
import { expectTypeOf } from "vitest";
import type { Run } from "./generated/handles.ts";
import type { Checkpoint, RunCheckpointParams } from "./generated/types.ts";

declare const run: Run;

expectTypeOf<Parameters<Run["checkpoint"]>>().toEqualTypeOf<[]>();
expectTypeOf<Awaited<ReturnType<Run["checkpoint"]>>>().toEqualTypeOf<Checkpoint>();
// @ts-expect-error a program names no path on the engine's machine
void run.checkpoint({ location: "/srv/checkpoints/nightly.sqlite" });
void run.migrate({ location: "/srv/checkpoints/nightly.sqlite" });
expectTypeOf<RunCheckpointParams["location"]>().toEqualTypeOf<string | null | undefined>();
