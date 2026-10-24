// The Console's formatting (spec #592's `format`; ADR 0073): every number
// and point in time a screen shows goes through here, formatted by `Intl`
// with the browser's locale and time zone unless a caller names others.
// Screens put the machine form beside it (`<data value>`, `<time
// datetime>`), which their tests assert; formatted text is asserted only in
// the `unit` project.

/** The millionths a price Limit counts its currency in (ADR 0041). */
const MILLIONTHS = 1_000_000;

/**
 * The amount of a Limit a person typed in `text`, as `formatAmount` takes
 * it: a whole count when `currency` is none, else the currency's units with
 * at most six decimals after a `.` or a `,`, in millionths; none for any
 * other text or an amount past what a number holds exactly.
 */
export function parseAmount(text: string, currency: string | null | undefined): number | undefined {
  const typed = text.trim();
  if (currency === undefined || currency === null) {
    return /^\d+$/u.test(typed) ? exact(Number(typed)) : undefined;
  }
  const parts = /^(?<units>\d+)(?:[.,](?<fraction>\d{1,6}))?$/u.exec(typed)?.groups;
  if (parts?.["units"] === undefined) {
    return undefined;
  }
  const fraction = (parts["fraction"] ?? "").padEnd(6, "0");
  return exact(Number(parts["units"]) * MILLIONTHS + Number(fraction));
}

/** `amount` when a number holds it exactly. */
function exact(amount: number): number | undefined {
  return Number.isSafeInteger(amount) ? amount : undefined;
}

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
  // A decimal string, which `Intl` formats exactly where a division by a
  // million would round the largest amounts.
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- digits, a dot and six digits: a numeric literal.
  const decimal = `${String(units)}.${millionths}` as Intl.StringNumericLiteral;
  return new Intl.NumberFormat(locales, {
    style: "currency",
    currency,
    maximumFractionDigits: 6,
  }).format(decimal);
}

/** The units a size is shown in, each 1,000 times the one before. */
const BYTE_UNITS = ["byte", "kilobyte", "megabyte", "gigabyte", "terabyte", "petabyte"] as const;

/** `bytes` in the largest unit it holds at least one of, to a tenth. */
export function formatBytes(bytes: number, locales?: Intl.LocalesArgument): string {
  let unit = 0;
  while (unit < BYTE_UNITS.length - 1 && bytes >= 1_000 ** (unit + 1)) {
    unit += 1;
  }
  return new Intl.NumberFormat(locales, {
    style: "unit",
    unit: BYTE_UNITS[unit],
    maximumFractionDigits: 1,
  }).format(bytes / 1_000 ** unit);
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
