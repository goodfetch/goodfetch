// The pinned lychee and the built site laid out as its host serves it, for
// the link check (`links.spec.ts`) and the Live check `docs-external-links`.
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { built } from "./site.ts";

const pins = join(import.meta.dirname, "../../test-support/lychee/pins.toml");

/** The tests' copy of the pinned lychee, which `ci/lychee-fetch.sh` writes. */
export function lychee(): string {
  const version = /^version = "([^"]+)"$/m.exec(readFileSync(pins, "utf8"))?.[1];
  if (version === undefined) {
    throw new Error(`${pins} pins no lychee version`);
  }
  const downloads =
    process.env["GOODFETCH_LYCHEE_DOWNLOADS"] ?? join(homedir(), ".cache/goodfetch/lychee");
  const executable = join(
    downloads,
    version,
    process.platform === "win32" ? "lychee.exe" : "lychee",
  );
  if (!existsSync(executable)) {
    throw new Error(`${executable} is missing: run ci/lychee-fetch.sh (part of \`just fetch\`)`);
  }
  return executable;
}

/**
 * Calls `check` with a folder holding a copy of the built site under
 * `docs/`, so that its absolute `/docs/...` links resolve as on
 * `goodfetch.dev`, and removes the folder after.
 */
export function withSiteCopy<T>(check: (root: string) => T): T {
  const root = mkdtempSync(join(tmpdir(), "goodfetch-docs-links-"));
  try {
    cpSync(built, join(root, "docs"), { recursive: true });
    return check(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
