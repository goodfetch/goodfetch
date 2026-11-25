// Uses each TypeScript escape oxlint must refuse; ci/proofs/run.sh expects one
// `no-explicit-any`, one `no-non-null-assertion` and two `ban-ts-comment`
// errors, and none for the suppression that says why.

export const anything: any = JSON.parse("1");

export function first(values: string[]): string {
  return values[0]!;
}

// @ts-ignore
export const ignored: number = "ignored";

// @ts-expect-error
export const unexplained: number = "unexplained";

// @ts-expect-error: a suppression with its reason is allowed.
export const explained: number = "explained";
