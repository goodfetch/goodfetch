export type TerminalStatus = 'verified' | 'failed' | 'unsupported' | 'timeout' | 'crashed' | 'rejected';
export type Sample = { status: TerminalStatus; elapsedMs: number | null };
export function quantiles(values: number[]) {
  const sorted = values.slice().sort((a, b) => a - b);
  const at = (q: number) => sorted.length ? sorted[Math.max(0, Math.ceil(q * sorted.length) - 1)]! : null;
  return { n: sorted.length, p50: at(.5), p95: at(.95), p99: at(.99) };
}
export function summarize(samples: Sample[]) {
  const count = (status: TerminalStatus) => samples.filter(row => row.status === status).length;
  for (const row of samples) if (row.elapsedMs !== null && (!Number.isFinite(row.elapsedMs) || row.elapsedMs < 0)) throw new Error('invalid sample duration');
  return {
    attempts: samples.length, verified: count('verified'), failed: count('failed'), unsupported: count('unsupported'),
    timeout: count('timeout'), crashed: count('crashed'), rejected: count('rejected'),
    successRate: samples.length ? count('verified') / samples.length : null,
    successLatencyMs: quantiles(samples.filter(row => row.status === 'verified' && row.elapsedMs !== null).map(row => row.elapsedMs!)),
    startedLatencyMs: quantiles(samples.filter(row => row.elapsedMs !== null).map(row => row.elapsedMs!)),
    zeroFailureUpper95: samples.length && samples.every(row => row.status === 'verified') ? 1 - Math.pow(.05, 1 / samples.length) : null,
    interpretation: samples.length < 1000 ? 'exploratory; insufficient tail and reliability evidence' : 'empirical quantiles; independence and representativeness not established'
  };
}
export function random(seed: number) {
  let state = seed >>> 0;
  return () => { state += 0x6D2B79F5; let t = state; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
}
export function shuffle<T>(items: T[], seed: number): T[] {
  const result = items.slice(), rng = random(seed);
  for (let i = result.length - 1; i > 0; i--) { const j = Math.floor(rng() * (i + 1)); [result[i], result[j]] = [result[j]!, result[i]!]; }
  return result;
}
