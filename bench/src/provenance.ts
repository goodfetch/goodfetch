// A run's provenance: the commit it ran on, whether the checkout had
// uncommitted changes, and a fingerprint of the harness's own sources. A run
// is reproducible only from a clean commit, so a dirty checkout caps its host
// at `local-dev`, and a source that changes during the run invalidates it.
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile, readdir } from "node:fs/promises";
import { join, relative, sep } from "node:path";
import { promisify } from "node:util";

const exec = promisify(execFile);

/** Folders under a source that hold no source: installs, builds and results. */
const notSources = new Set(["node_modules", "target", "results"]);

/** Where a run came from, as its record writes it. */
export interface Provenance {
  /** The commit and the checkout's state, or `null` outside a git checkout. */
  readonly git: { readonly sha: string; readonly dirty: boolean } | null;
  /** SHA-256 over every source file's path and SHA-256. */
  readonly source_fingerprint: string;
  /** Each source file's SHA-256, by its path from the repository's root. */
  readonly source_files: Readonly<Record<string, string>>;
  readonly node: string;
}

async function git(repository: string, args: readonly string[]): Promise<string | null> {
  try {
    return (await exec("git", args, { cwd: repository, timeout: 10_000 })).stdout.trim();
  } catch {
    // Not a git checkout, or no git: the record says so with `git: null`.
    return null;
  }
}

async function sourceFiles(
  repository: string,
  sources: readonly string[],
): Promise<Record<string, string>> {
  const paths: string[] = [];
  for (const source of sources) {
    for (const entry of await readdir(join(repository, source), {
      recursive: true,
      withFileTypes: true,
    })) {
      const path = relative(repository, join(entry.parentPath, entry.name)).split(sep).join("/");
      if (entry.isFile() && !path.split("/").some((part) => notSources.has(part))) {
        paths.push(path);
      }
    }
  }
  const files: Record<string, string> = {};
  for (const path of paths.toSorted()) {
    files[path] = createHash("sha256")
      .update(await readFile(join(repository, path)))
      .digest("hex");
  }
  return files;
}

/** Reads the provenance of `repository`, fingerprinting the folders `sources` names. */
export async function captureProvenance(input: {
  readonly repository: string;
  readonly sources: readonly string[];
}): Promise<Provenance> {
  const sha = await git(input.repository, ["rev-parse", "HEAD"]);
  const status = sha === null ? null : await git(input.repository, ["status", "--porcelain"]);
  const files = await sourceFiles(input.repository, input.sources);
  return {
    git: sha === null || status === null ? null : { sha, dirty: status.length > 0 },
    source_fingerprint: createHash("sha256")
      .update(JSON.stringify(Object.entries(files)))
      .digest("hex"),
    source_files: files,
    node: process.version,
  };
}

/** The source files added, removed or changed between two captures, sorted. */
export function changedSources(before: Provenance, after: Provenance): string[] {
  const paths = new Set([...Object.keys(before.source_files), ...Object.keys(after.source_files)]);
  return [...paths]
    .filter((path) => before.source_files[path] !== after.source_files[path])
    .toSorted();
}
