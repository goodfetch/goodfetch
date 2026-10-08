// Quantiles and the two one-sided tests (TOST) for equivalence of a paired mean difference.

// Acklam's rational approximation of the inverse normal CDF, relative error below 1.2e-9.
export function normalQuantile(p: number): number {
  if (!(p > 0 && p < 1)) throw new Error('normal quantile requires 0 < p < 1');
  const a = [-3.969683028665376e1, 2.209460984245205e2, -2.759285104469687e2, 1.383577518672690e2, -3.066479806614716e1, 2.506628277459239];
  const b = [-5.447609879822406e1, 1.615858368580409e2, -1.556989798598866e2, 6.680131188771972e1, -1.328068155288572e1];
  const c = [-7.784894002430293e-3, -3.223964580411365e-1, -2.400758277161838, -2.549732539343734, 4.374664141464968, 2.938163982698783];
  const d = [7.784695709041462e-3, 3.224671290700398e-1, 2.445134137142996, 3.754408661907416];
  const tail = (q: number) => (((((c[0]! * q + c[1]!) * q + c[2]!) * q + c[3]!) * q + c[4]!) * q + c[5]!) / ((((d[0]! * q + d[1]!) * q + d[2]!) * q + d[3]!) * q + 1);
  if (p < 0.02425) return tail(Math.sqrt(-2 * Math.log(p)));
  if (p > 1 - 0.02425) return -tail(Math.sqrt(-2 * Math.log(1 - p)));
  const q = p - 0.5, r = q * q;
  return (((((a[0]! * r + a[1]!) * r + a[2]!) * r + a[3]!) * r + a[4]!) * r + a[5]!) * q / (((((b[0]! * r + b[1]!) * r + b[2]!) * r + b[3]!) * r + b[4]!) * r + 1);
}

function logGamma(x: number): number {
  // Lanczos approximation (g = 7, n = 9).
  const g = [0.99999999999980993, 676.5203681218851, -1259.1392167224028, 771.32342877765313, -176.61502916214059, 12.507343278686905, -0.13857109526572012, 9.9843695780195716e-6, 1.5056327351493116e-7];
  if (x < 0.5) return Math.log(Math.PI / Math.sin(Math.PI * x)) - logGamma(1 - x);
  x -= 1; let sum = g[0]!; for (let i = 1; i < 9; i++) sum += g[i]! / (x + i);
  const t = x + 7.5;
  return 0.5 * Math.log(2 * Math.PI) + (x + 0.5) * Math.log(t) - t + Math.log(sum);
}

// Regularized incomplete beta I_x(a, b) by Lentz's continued fraction.
function incompleteBeta(x: number, a: number, b: number): number {
  if (x <= 0) return 0; if (x >= 1) return 1;
  const front = Math.exp(logGamma(a + b) - logGamma(a) - logGamma(b) + a * Math.log(x) + b * Math.log(1 - x));
  if (x > (a + 1) / (a + b + 2)) return 1 - incompleteBeta(1 - x, b, a);
  let f = 1, c = 1, d = 1 - (a + b) * x / (a + 1); d = 1 / (Math.abs(d) < 1e-300 ? 1e-300 : d); f = d;
  for (let m = 1; m <= 300; m++) {
    for (const numerator of [m * (b - m) * x / ((a + 2 * m - 1) * (a + 2 * m)), -(a + m) * (a + b + m) * x / ((a + 2 * m) * (a + 2 * m + 1))]) {
      d = 1 + numerator * d; d = 1 / (Math.abs(d) < 1e-300 ? 1e-300 : d);
      c = 1 + numerator / c; if (Math.abs(c) < 1e-300) c = 1e-300;
      f *= c * d;
    }
    if (Math.abs(c * d - 1) < 1e-14) break;
  }
  return front * f / a;
}

export function studentTCdf(t: number, df: number): number {
  const tail = 0.5 * incompleteBeta(df / (df + t * t), df / 2, 0.5);
  return t >= 0 ? 1 - tail : tail;
}

export function studentTQuantile(p: number, df: number): number {
  if (!(p > 0 && p < 1) || !(df > 0)) throw new Error('invalid Student t quantile arguments');
  let low = -1e4, high = 1e4;
  for (let i = 0; i < 200; i++) { const mid = (low + high) / 2; if (studentTCdf(mid, df) < p) low = mid; else high = mid; }
  return (low + high) / 2;
}

// TOST at level alpha: equivalent when the (1 - 2 alpha) t interval of the mean lies inside (-margin, +margin).
export function tost(differences: number[], margin: number, alpha: number) {
  const n = differences.length;
  if (n < 2) throw new Error('TOST needs at least two paired differences');
  const mean = differences.reduce((a, b) => a + b, 0) / n;
  const sd = Math.sqrt(differences.reduce((sum, d) => sum + (d - mean) ** 2, 0) / (n - 1));
  const half = studentTQuantile(1 - alpha, n - 1) * sd / Math.sqrt(n);
  const ci: [number, number] = [mean - half, mean + half];
  return { n, mean, sd, ci, margin, equivalent: ci[0] > -margin && ci[1] < margin };
}
