// The folder each of test support's own tests of its runners copies a
// runner's configuration into. It sits in the checkout, beside where the
// configuration lives, so the copy finds the repository's `package.json`
// (`"type": "module"`) and `node_modules` as the original does. It is never
// under `target/`, which a worktree may link to a folder outside the
// repository, where Node finds neither and refuses the configuration as an
// ES module. Git ignores the folder, and Vitest and Playwright skip the dot
// folders at the repository's root.
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { onTestFinished } from "vitest";

const PROJECTS = join(import.meta.dirname, "..", "..", "..", ".runner-projects");

/**
 * Makes a new folder whose name starts with `prefix` for the running test,
 * and returns its path; the folder goes when the test ends.
 */
export function runnerProject(prefix: string): string {
  mkdirSync(PROJECTS, { recursive: true });
  const folder = mkdtempSync(join(PROJECTS, `${prefix}-`));
  onTestFinished(() => rmSync(folder, { recursive: true, force: true }));
  return folder;
}
