// node:test reporter that writes one block per failing test: its location and name, then the first lines of its error.
// A file that crashes before registering tests fails as a whole with a bare "test failed"; its first error line on
// stderr is reported instead. The local CI prints this file under a failed step so one run identifies a flake.
import { relative, resolve } from 'node:path';

type Event = { type: string; data: { name?: string; file?: string; line?: number; message?: string; details?: { error?: Error & { failureType?: string; cause?: unknown } } } };

const firstLines = (text: string, count = 3) => text.split('\n').map(line => line.trimEnd()).filter(Boolean).slice(0, count);

export default async function* failures(source: AsyncIterable<Event>) {
  const stderr = new Map<string, string[]>();
  for await (const { type, data } of source) {
    if (type === 'test:stderr' && data.file) {
      const file = resolve(data.file);
      stderr.set(file, [...stderr.get(file) ?? [], ...(data.message ?? '').split('\n')]);
    }
    if (type !== 'test:fail' || !data.file) continue;
    const error = data.details?.error;
    if (error?.failureType === 'subtestsFailed') continue; // the failing subtest has its own block
    const file = resolve(data.file), location = `${relative(process.cwd(), file)}:${data.line ?? 1}`;
    const cause = error?.cause instanceof Error ? error.cause.message : String(error?.cause ?? error?.message ?? '');
    const crashed = cause === 'test failed' && data.name !== undefined && resolve(data.name) === file;
    const detail = crashed ? (stderr.get(file) ?? []).filter(line => /^\w*Error\b/.test(line)).slice(0, 1) : firstLines(cause);
    yield [`${location} ${crashed ? '(file failed)' : data.name}`, ...detail.map(line => `    ${line}`)].join('\n') + '\n';
  }
}
