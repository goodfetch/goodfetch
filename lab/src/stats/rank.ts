// Rank statistics for paired block differences.
export function wilcoxonSignedRank(differences: number[]): { n: number; wPlus: number; pValue: number; method: 'exact' | 'normal' } {
  const nonZero = differences.filter(d => d !== 0);
  const n = nonZero.length;
  if (!n) return { n: 0, wPlus: 0, pValue: 1, method: 'exact' };
  const sorted = nonZero.map((d, index) => ({ abs: Math.abs(d), positive: d > 0, index })).sort((a, b) => a.abs - b.abs);
  const ranks = new Array<number>(n);
  let ties = false;
  for (let i = 0; i < n;) {
    let j = i; while (j + 1 < n && sorted[j + 1]!.abs === sorted[i]!.abs) j++;
    if (j > i) ties = true;
    for (let k = i; k <= j; k++) ranks[k] = (i + j) / 2 + 1;
    i = j + 1;
  }
  const wPlus = sorted.reduce((sum, item, k) => sum + (item.positive ? ranks[k]! : 0), 0);
  const total = n * (n + 1) / 2;
  if (!ties && n <= 25) {
    // Exact null distribution of W+ by counting sign assignments.
    const counts = new Array<number>(total + 1).fill(0); counts[0] = 1;
    for (let rank = 1; rank <= n; rank++) for (let s = total; s >= rank; s--) counts[s]! += counts[s - rank]!;
    const all = 2 ** n;
    const lower = counts.slice(0, Math.floor(wPlus) + 1).reduce((a, b) => a + b, 0) / all;
    const upper = counts.slice(Math.ceil(wPlus)).reduce((a, b) => a + b, 0) / all;
    return { n, wPlus, pValue: Math.min(1, 2 * Math.min(lower, upper)), method: 'exact' };
  }
  const mean = total / 2;
  let tieCorrection = 0;
  for (let i = 0; i < n;) { let j = i; while (j + 1 < n && sorted[j + 1]!.abs === sorted[i]!.abs) j++; const t = j - i + 1; tieCorrection += t * t * t - t; i = j + 1; }
  const variance = n * (n + 1) * (2 * n + 1) / 24 - tieCorrection / 48;
  const z = (Math.abs(wPlus - mean) - 0.5) / Math.sqrt(variance);
  return { n, wPlus, pValue: Math.min(1, 2 * (1 - normalCdf(Math.max(0, z)))), method: 'normal' };
}

// One-sample Hodges-Lehmann location estimate: median of the Walsh averages (d_i + d_j) / 2, i <= j.
export function hodgesLehmann(values: number[]): number {
  const walsh: number[] = [];
  for (let i = 0; i < values.length; i++) for (let j = i; j < values.length; j++) walsh.push((values[i]! + values[j]!) / 2);
  walsh.sort((a, b) => a - b);
  const middle = Math.floor(walsh.length / 2);
  return walsh.length % 2 ? walsh[middle]! : (walsh[middle - 1]! + walsh[middle]!) / 2;
}

export function normalCdf(x: number): number {
  // Abramowitz-Stegun 7.1.26 applied to erf; absolute error below 1.5e-7.
  const t = 1 / (1 + 0.3275911 * Math.abs(x) / Math.SQRT2);
  const erf = 1 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * Math.exp(-x * x / 2);
  return x >= 0 ? (1 + erf) / 2 : (1 - erf) / 2;
}
