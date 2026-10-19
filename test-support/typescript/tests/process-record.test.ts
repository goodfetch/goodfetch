// Seam B (spec #325): the process record (ADR 0069) fails a test whose
// process is alive after teardown, and passes one whose processes ended.
import { describe, expect } from "vitest";
import { ProcessRecord, test } from "@goodfetch/test-support";

const STAYS_ALIVE = ["-e", "setInterval(() => {}, 1000)"];
const EXITS = ["-e", "process.exitCode = 0"];

describe("ProcessRecord", () => {
  test("closes once every recorded process has ended", async () => {
    const processes = new ProcessRecord();
    const exited = processes.spawn(process.execPath, EXITS, { env: process.env });
    await exited.exit;

    const closed = processes.close();

    await expect(closed).resolves.toBeUndefined();
  });

  test("fails with a process still alive, and ends it", async () => {
    const processes = new ProcessRecord();
    const alive = processes.spawn(process.execPath, STAYS_ALIVE, { env: process.env });

    const closed = processes.close();

    await expect(closed).rejects.toMatchObject({ code: "processes_alive" });
    await expect(alive.exit).resolves.toMatchObject({ signal: "SIGKILL" });
  });
});
