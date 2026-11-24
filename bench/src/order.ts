// The seeded order of a run and the quantiles of its figures, carried over
// from the lab's statistics: the same seed always gives the same order, so a
// run's randomized blocks can be replayed.

/** A generator of numbers in [0, 1) from `seed` (mulberry32). */
export function random(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state += 0x6d_2b_79_f5;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4_294_967_296;
  };
}

/** `items` in the order `seed` draws (Fisher–Yates). */
export function shuffle<T>(items: readonly T[], seed: number): T[] {
  const result = [...items];
  const next = random(seed);
  for (let index = result.length - 1; index > 0; index -= 1) {
    const other = Math.floor(next() * (index + 1));
    const here = result[index];
    const there = result[other];
    if (here === undefined || there === undefined) {
      throw new Error("invariant: both indices are inside the array");
    }
    result[index] = there;
    result[other] = here;
  }
  return result;
}

/** The nearest-rank `q` quantile of `values`, or `null` when there are none. */
export function quantile(values: readonly number[], q: number): number | null {
  const sorted = values.toSorted((left, right) => left - right);
  return sorted[Math.max(0, Math.ceil(q * sorted.length) - 1)] ?? null;
}
