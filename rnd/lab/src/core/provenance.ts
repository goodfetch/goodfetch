import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { labRoot } from '../registry.ts';
import { sourceFingerprint } from '../environment.ts';

const exec = promisify(execFile);
async function output(command: string, args: string[], cwd = labRoot) { try { return (await exec(command, args, { cwd, timeout: 5000 })).stdout.trim(); } catch { return null; } }

export type Provenance = {
  git: { available: boolean; sha: string | null; dirty: boolean | null };
  sourceTreeSha256: string | null; sourceHashes: Record<string, string>; toolchains: Record<string, string | null>;
};

// A run is reproducible only from a commit: uncommitted lab sources cap the host class at local-dev.
export async function captureProvenance(): Promise<Provenance> {
  const sha = await output('git', ['rev-parse', 'HEAD']);
  const status = sha ? await output('git', ['status', '--porcelain', '--', '.']) : null;
  const sourceHashes = await sourceFingerprint();
  const tree = createHash('sha256').update(JSON.stringify(Object.entries(sourceHashes).sort(([a], [b]) => a.localeCompare(b)))).digest('hex');
  return {
    git: { available: sha !== null, sha, dirty: status === null ? null : status.length > 0 },
    sourceTreeSha256: tree, sourceHashes,
    toolchains: { node: process.version, rustc: await output('rustc', ['--version']), go: await output(join(labRoot, '.toolchains/go/bin/go'), ['version']), pnpm: await output('pnpm', ['--version']) }
  };
}
