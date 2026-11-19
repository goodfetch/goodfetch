// Ctrl+C reaches only the host (ADRs 0078 and 0093): its Attached engine
// runs in a process group of its own, so SIGINT sent to the host's group
// leaves the engine running, and the host's handler, as the SDK's docs
// advise, closes the Run, which stops it with a Checkpoint, then the
// Connection. Played against the Scripted engine; Python's
// `test_forks_and_interrupts.py` is its twin.
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { setTimeout } from "node:timers/promises";
import { scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** The methods of the Client's messages the Scripted engine recorded under `home`. */
function clientMethods(home: string): unknown[] {
  return readFileSync(join(home, "scripted-engine", "record.jsonl"), "utf8")
    .split("\n")
    .filter((line) => line !== "")
    .map((line): unknown => JSON.parse(line))
    .filter(
      (entry): entry is { from: string; message: { method?: unknown } } =>
        typeof entry === "object" && entry !== null && "from" in entry && entry.from === "client",
    )
    .map((entry) => entry.message.method);
}

// Its signal is SIGINT; Windows's CTRL_C_EVENT has no test yet.
describe.skipIf(process.platform === "win32")(
  "Ctrl+C sent to a host whose Run runs on an Attached engine",
  () => {
    test(
      "reaches the host alone, whose handler closes the Run with a Checkpoint, then the Connection",
      { tags: ["client_case/interrupt-reaches-only-the-host", "attached"] },
      async ({ testEnv, processes, clientCase }) => {
        const env = scriptedEngineEnv(testEnv, "collie", "run/closed-while-running.json");
        const out = join(testEnv.folder, "interrupted.json");
        const running = join(testEnv.folder, "running");
        // The host's own group, as a terminal gives the program it runs; the
        // catalog's marker warnings, which Node prints, are the host's to silence.
        const host = processes.spawn(
          process.execPath,
          ["--no-warnings", join(import.meta.dirname, "fixtures", "interrupted.ts"), out, running],
          { env, cwd: testEnv.folder, detached: true },
        );
        const stdout: Buffer[] = [];
        const stderr: Buffer[] = [];
        host.child.stdout.on("data", (bytes: Buffer) => stdout.push(bytes));
        host.child.stderr.on("data", (bytes: Buffer) => stderr.push(bytes));
        host.child.stdin.end();
        const ended = host.exit.then(() => true);
        while (!existsSync(running)) {
          if (await Promise.race([ended, setTimeout(50, false)])) {
            throw new Error(
              `the host ended before its Run ran: ${Buffer.concat(stderr).toString()}`,
            );
          }
        }

        // Ctrl+C: the terminal signals the whole foreground group.
        const pid = host.child.pid;
        if (pid === undefined) {
          throw new TypeError("invariant: a host that created its file has started");
        }
        process.kill(-pid, "SIGINT");
        const exit = await host.exit;

        await clientCase.check("the host's handler got SIGINT and the host exited 0", () => {
          expect(exit).toMatchObject({ code: 0, signal: null });
          expect(JSON.parse(readFileSync(out, "utf8"))).toMatchObject({ signal: "SIGINT" });
        });
        await clientCase.check(
          "the engine, in a group of its own, got the Run's close, which stopped it with a Checkpoint, and played its script to its end",
          () => {
            expect(clientMethods(testEnv.home)).toEqual(["hello", "run.start", "run.abort"]);
            expect(JSON.parse(readFileSync(out, "utf8"))).toMatchObject({ warned: [] });
          },
        );
        await clientCase.check("the SDK wrote nothing to the host's stdout or stderr", () =>
          expect({
            stdout: Buffer.concat(stdout).toString(),
            stderr: Buffer.concat(stderr).toString(),
          }).toEqual({ stdout: "", stderr: "" }),
        );
      },
    );
  },
);
