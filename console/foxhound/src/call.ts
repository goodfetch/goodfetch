// A call of an operation only Foxhound carries, typed by Foxhound's
// Console's operation map, which `xtask codegen` writes from the catalog
// (spec #593; ADR 0099): the Console's Client sends it as a read, again on
// the next Connection when its own drops. Engine answers are not validated
// beyond the protocol version check (ADR 0038).
import type { ConsoleClient } from "@goodfetch/console/client";
import type { FoxhoundConsoleOperations } from "#generated/operations.ts";

/** Calls `operation` with `params` over `client`, and answers its result. */
export async function callFoxhound<Name extends keyof FoxhoundConsoleOperations>(
  client: ConsoleClient,
  operation: Name,
  params: FoxhoundConsoleOperations[Name]["params"],
  signal: AbortSignal,
): Promise<FoxhoundConsoleOperations[Name]["result"]> {
  const result = await client.callFoxhound(operation, { ...params }, { signal });
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the engine answers the catalog's result type, which the Console does not validate again (ADR 0038).
  return result as FoxhoundConsoleOperations[Name]["result"];
}
