// The backoff between attempts to reach a remote Daemon again after its
// link dropped (spec 606, ADR 0079): 250 ms before the first, doubling each
// time, capped at 10 s, and no attempt starting once `timeout.connect`,
// counted from the drop, has passed.

/** The wait before the first attempt, in milliseconds. */
const FIRST_DELAY = 250;

/** The longest wait between two attempts, in milliseconds. */
const LONGEST_DELAY = 10_000;

/**
 * The milliseconds to wait before attempt `attempt`, the first being 0,
 * `elapsed` milliseconds after the drop; `undefined` once that attempt
 * would start at or past `within`, the `timeout.connect` in force, which
 * `null` leaves unbounded.
 */
export function reconnectDelay(
  attempt: number,
  elapsed: number,
  within: number | null,
): number | undefined {
  const delay = Math.min(FIRST_DELAY * 2 ** attempt, LONGEST_DELAY);
  return within !== null && elapsed + delay >= within ? undefined : delay;
}
