// The Connection's types (ADRs 0095 and 0099): a helper a library serving
// both Products writes, typed with the client package's `Connection`, takes
// a `Collie` and a `Foxhound`, and refuses any other value; `Connection`
// carries exactly the members both Products share. Python's
// `tests/types/connection_types.py` and Rust's `tests/ui/connection/` are its
// twins.
import type { Collie } from "@goodfetch/collie";
import type { Connection, EngineInfo, Run } from "@goodfetch/collie-client";
import { expectTypeOf } from "vitest";
import type { Foxhound } from "../src/index.ts";

/** A library's helper, written once for either Product. */
declare function startOn(connection: Connection): Promise<Run>;
declare const collie: Collie;
declare const foxhound: Foxhound;
declare const run: Run;
declare const engine: EngineInfo;

expectTypeOf(startOn).toBeCallableWith(collie);
expectTypeOf(startOn).toBeCallableWith(foxhound);
// @ts-expect-error -- a Run is no Connection.
void startOn(run);
// @ts-expect-error -- nor is a value carrying only `engine()`.
void startOn({ engine: () => engine });

expectTypeOf<keyof Connection>().toEqualTypeOf<
  | "engine"
  | "start"
  | "startRecipe"
  | "startCatalogRun"
  | "openRun"
  | "call"
  | "explainSettings"
  | "close"
  | typeof Symbol.asyncDispose
>();
