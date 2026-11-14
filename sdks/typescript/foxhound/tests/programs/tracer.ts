// The tracer program: one Run started and finished on a Connection, which
// it then closes. It is typed for `Connection`, the type both Products'
// Connections implement (ADR 0095), so the same helper takes a `Foxhound`
// and a `Collie`.
import type { Connection, RunReport } from "@goodfetch/collie-client";

/** What the tracer saw. */
export interface Traced {
  readonly startedAs: string;
  readonly runId: string;
  readonly report: RunReport;
}

/** Starts a Run on `connection`, finishes it, then closes the Connection. */
export async function startAndFinish(connection: Connection): Promise<Traced> {
  const run = await connection.start();
  const report = await run.finish();
  await connection.close();
  return { startedAs: run.startedAs(), runId: run.id(), report };
}
