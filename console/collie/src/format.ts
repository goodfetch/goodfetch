// The Console's formatting (spec #592's `format`; ADR 0073): every number
// and point in time a screen shows goes through here, formatted by `Intl`
// with the browser's locale and time zone unless a caller names others.
// Screens put the machine form beside it (`<data value>`, `<time
// datetime>`), which their tests assert; formatted text is asserted only in
// the `unit` project.

/** The millionths a price Limit counts its currency in (ADR 0041). */
const MILLIONTHS = 1_000_000;

/** `count`, grouped as `locales` group digits. */
export function formatCount(count: number, locales?: Intl.LocalesArgument): string {
  return new Intl.NumberFormat(locales).format(count);
}

/**
 * `amount` of a Limit or of Usage: a count when `currency` is none, else
 * millionths of `currency`, an ISO 4217 code, shown exactly, with at least
 * the currency's usual decimals.
 */
export function formatAmount(
  amount: number,
  currency: string | null | undefined,
  locales?: Intl.LocalesArgument,
): string {
  if (currency === undefined || currency === null) {
    return formatCount(amount, locales);
  }
  const units = Math.trunc(amount / MILLIONTHS);
  const millionths = String(amount % MILLIONTHS).padStart(6, "0");
  return new Intl.NumberFormat(locales, {
    style: "currency",
    currency,
    maximumFractionDigits: 6,
  }).format(`${String(units)}.${millionths}`);
}

/** The point in time `rfc3339` on the wall clock of `timeZone`, the browser's by default. */
export function formatInstant(
  rfc3339: string,
  locales?: Intl.LocalesArgument,
  timeZone?: string,
): string {
  return new Intl.DateTimeFormat(locales, {
    dateStyle: "medium",
    timeStyle: "short",
    ...(timeZone === undefined ? {} : { timeZone }),
  }).format(new Date(rfc3339));
}
