// A run's provenance: the commit it ran on, whether the checkout had
// uncommitted changes, and a fingerprint of bench's own sources, whose change
// during the run the record reports.
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, test } from "vitest";
import { captureProvenance, changedSources } from "./provenance.ts";

const folders: string[] = [];
afterEach(() => {
  for (const folder of folders.splice(0)) {
    rmSync(folder, { recursive: true, force: true });
  }
});

function repository(): string {
  const folder = mkdtempSync(join(tmpdir(), "goodfetch-bench-provenance-"));
  folders.push(folder);
  const git = (...args: string[]): void => {
    execFileSync("git", ["-c", "user.name=Bench", "-c", "user.email=bench@example.test", ...args], {
      cwd: folder,
      stdio: "ignore",
    });
  };
  git("init", "-q");
  mkdirSync(join(folder, "bench", "src"), { recursive: true });
  writeFileSync(join(folder, "bench", "src", "runner.ts"), "export const a = 1;\n");
  writeFileSync(join(folder, "README.md"), "Elsewhere\n");
  git("add", ".");
  git("commit", "-q", "-m", "first");
  return folder;
}

test("records the commit, a clean checkout and the sources' fingerprint", async () => {
  const folder = repository();
  const provenance = await captureProvenance({ repository: folder, sources: ["bench"] });
  expect(provenance.git).toEqual({ sha: expect.stringMatching(/^[0-9a-f]{40}$/), dirty: false });
  expect(Object.keys(provenance.source_files)).toEqual(["bench/src/runner.ts"]);
  expect(provenance.source_fingerprint).toMatch(/^[0-9a-f]{64}$/);
});

test("marks any uncommitted change dirty, and only a source's changes the fingerprint", async () => {
  const folder = repository();
  const before = await captureProvenance({ repository: folder, sources: ["bench"] });

  writeFileSync(join(folder, "README.md"), "Changed elsewhere\n");
  const elsewhere = await captureProvenance({ repository: folder, sources: ["bench"] });
  expect(elsewhere.git?.dirty).toBe(true);
  expect(elsewhere.source_fingerprint).toBe(before.source_fingerprint);

  writeFileSync(join(folder, "bench", "src", "runner.ts"), "export const a = 2;\n");
  writeFileSync(join(folder, "bench", "src", "new.ts"), "export const b = 1;\n");
  const changed = await captureProvenance({ repository: folder, sources: ["bench"] });
  expect(changed.source_fingerprint).not.toBe(before.source_fingerprint);
  expect(changedSources(before, changed)).toEqual(["bench/src/new.ts", "bench/src/runner.ts"]);
});

test("records no commit outside a git checkout", async () => {
  const folder = mkdtempSync(join(tmpdir(), "goodfetch-bench-provenance-"));
  folders.push(folder);
  mkdirSync(join(folder, "bench"));
  writeFileSync(join(folder, "bench", "runner.ts"), "export const a = 1;\n");
  const provenance = await captureProvenance({ repository: folder, sources: ["bench"] });
  expect(provenance.git).toBeNull();
  expect(Object.keys(provenance.source_files)).toEqual(["bench/runner.ts"]);
});
