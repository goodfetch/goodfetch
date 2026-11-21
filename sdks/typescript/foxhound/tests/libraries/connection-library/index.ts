// A library serving both Products (story 11-180, ADR 0095): its
// `package.json` takes the Collie client package as a peer dependency, so an
// install gives it no copy of its own and its import resolves to the
// program's; in this workspace, its development link names that one package
// too. Its `instanceof` on the client package's `ClosedError` catches what a
// program's Connection fails with only while both share that one copy. Rust's
// `connection-library` crate and Python's `connection_library` are its twins.
import { ClosedError, type Connection, type RunReport } from "@goodfetch/collie-client";

/** Starts one Run on `connection` and finishes it, or answers `"closed"` once the program has closed it. */
export async function finishOneRun(connection: Connection): Promise<RunReport | "closed"> {
  try {
    const run = await connection.start();
    return await run.finish();
  } catch (error) {
    if (error instanceof ClosedError) {
      return "closed";
    }
    throw error;
  }
}
