// Foxhound's npm package finds its executable in the platform package
// installed beside it (ADR 0090): a project laid out as a package manager
// leaves it, whose platform package holds the Scripted engine under
// Foxhound's name, runs a program that sets no engine-path variable of
// Foxhound's. The packages packed and installed by each package manager
// wait for issue 3647. Python's `test_platform_wheel.py` is its twin.
import {
  chmodSync,
  copyFileSync,
  cpSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { scriptedEngineEnv, test, toolPath } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const PACKAGE = dirname(import.meta.dirname);

/**
 * Lays out the project `project`: Foxhound's package and the Collie client
 * package linked to their sources, as Node strips no types under
 * `node_modules`, and Foxhound's platform package for this system, holding
 * `engine`, the path of which it returns.
 */
function installedProject(project: string, engine: string): string {
  const scope = join(project, "node_modules", "@goodfetch");
  mkdirSync(scope, { recursive: true });
  const sources = join(project, "foxhound");
  mkdirSync(sources);
  copyFileSync(join(PACKAGE, "package.json"), join(sources, "package.json"));
  cpSync(join(PACKAGE, "src"), join(sources, "src"), { recursive: true });
  symlinkSync(sources, join(scope, "foxhound"), "junction");
  symlinkSync(
    realpathSync(join(PACKAGE, "node_modules", "@goodfetch", "collie-client")),
    join(scope, "collie-client"),
    "junction",
  );
  const name = `foxhound-${process.platform}-${process.arch}`;
  const bin = join(scope, name, "bin");
  mkdirSync(bin, { recursive: true });
  writeFileSync(
    join(scope, name, "package.json"),
    JSON.stringify({ name: `@goodfetch/${name}`, version: "0.0.0" }),
  );
  const executable = join(
    bin,
    process.platform === "win32" ? "goodfetch-foxhound.exe" : "goodfetch-foxhound",
  );
  copyFileSync(engine, executable);
  chmodSync(executable, 0o755);
  return executable;
}

describe("@goodfetch/foxhound installed with its platform package", () => {
  test(
    "starts the executable the platform package holds, whatever Collie's engine-path variable names",
    { tags: ["story:11-1001", "story:11-1007"] },
    async ({ testEnv, processes }) => {
      // The Scripted engine's map names its engine for Collie alone.
      const env: Readonly<Record<string, string>> = scriptedEngineEnv(
        testEnv,
        "collie",
        "run/foxhound-started-and-finished.json",
      );
      const project = join(realpathSync(testEnv.folder), "project");
      const executable = installedProject(project, toolPath(testEnv.tools, "scripted_engine"));
      const program = join(project, "program.ts");
      copyFileSync(join(import.meta.dirname, "fixtures", "installed.ts"), program);
      writeFileSync(join(project, "package.json"), JSON.stringify({ type: "module" }));
      const out = join(project, "reached.json");

      const run = processes.spawn(process.execPath, ["--no-warnings", program, out], {
        env: { ...env, GOODFETCH_COLLIE_ENGINE_PATH: "/nonexistent/goodfetch-collie" },
        cwd: project,
      });
      const stderr: Buffer[] = [];
      run.child.stderr.on("data", (bytes: Buffer) => stderr.push(bytes));
      run.child.stdin.end();
      const exit = await run.exit;

      expect({ exit, stderr: Buffer.concat(stderr).toString() }).toEqual({
        exit: { code: 0, signal: null },
        stderr: "",
      });
      expect(JSON.parse(readFileSync(out, "utf8"))).toEqual({
        kind: "attached",
        product: "foxhound",
        enginePath: executable,
        lifecycle: "finished",
      });
    },
  );
});
