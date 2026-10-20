// The tracer program (#1216): one Run started and finished on a Connection,
// which it then closes. The unit tests run it on the Scripted engine over
// the in-memory transport, the integration tests on an Attached engine of
// the executable of the same commit (ADR 0069): the same code on both.
import type { Collie, RunReport } from "@goodfetch/collie";

/** What the tracer saw. */
export interface Traced {
  readonly startedAs: string;
  readonly runId: string;
  readonly report: RunReport;
}

/** Starts a Run on `collie`, finishes it, then closes the Connection. */
export async function startAndFinish(collie: Collie): Promise<Traced> {
  const run = await collie.start();
  const report = await run.finish();
  await collie.close();
  return { startedAs: run.startedAs(), runId: run.id(), report };
}
