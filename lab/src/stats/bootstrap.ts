import { random } from '../statistics.ts';

// Percentile bootstrap of the mean over blocks: blocks are resampled whole, never the attempts inside them.
export function blockBootstrapCI(values: number[], { resamples = 10_000, seed, alpha = 0.05 }: { resamples?: number; seed: number; alpha?: number }) {
  if (!values.length) throw new Error('bootstrap needs at least one block');
  const n = values.length, rng = random(seed), means = new Array<number>(resamples);
  for (let r = 0; r < resamples; r++) { let sum = 0; for (let i = 0; i < n; i++) sum += values[Math.floor(rng() * n)]!; means[r] = sum / n; }
  means.sort((a, b) => a - b);
  const at = (q: number) => means[Math.min(resamples - 1, Math.max(0, Math.floor(q * resamples)))]!;
  return { estimate: values.reduce((a, b) => a + b, 0) / n, ci: [at(alpha / 2), at(1 - alpha / 2)] as [number, number], resamples, seed };
}
