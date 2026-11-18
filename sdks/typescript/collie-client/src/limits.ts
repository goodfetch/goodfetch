// Limits in code (ADRs 0041 and 0080): what `connect()` and a Run's start
// take, typed per Usage kind, with a price as an exact `Money`. The SDK
// turns them into the wire's list, `{ usage_kind, currency?, amount }` with
// a price in millionths of its currency, before anything is sent, and an
// amount it cannot turn exactly fails the call that carries it. No member
// raises a Limit: only a person does.
import { SettingInvalidError, type RunLimit } from "./generated/index.ts";

/**
 * A price: a decimal amount, a string so that no float rounding reaches a
 * cap, and its currency, such as `{ amount: "0.50", currency: "USD" }`.
 */
export interface Money {
  /** Digits with at most one decimal point, exact to a millionth of the currency. */
  readonly amount: string;
  /** An ISO 4217 code, such as `USD`. */
  readonly currency: string;
}

/**
 * The Usage kinds a Limit caps, each as a count of it. `paidFetches` is
 * Foxhound's: a Collie engine refuses it with `not_in_product`.
 */
interface Counts {
  /** Model tokens, input and output together. */
  readonly modelTokens?: number | undefined;
  /** Model calls. */
  readonly modelCalls?: number | undefined;
  /** Browser time, in milliseconds. */
  readonly browserTime?: number | undefined;
  /** Bytes through a proxy. */
  readonly proxyBytes?: number | undefined;
  /** Paid challenge solves. */
  readonly paidSolves?: number | undefined;
  /** Requests. */
  readonly requests?: number | undefined;
  /** Page crashes. */
  readonly pageCrashes?: number | undefined;
}

/**
 * The Limits a program sets on its Client connection at `connect()` or on
 * a Run at its start (ADR 0080): a count per Usage kind, and a price per
 * Usage kind as `Money`. Each only lowers what covers that work.
 */
export interface Limits extends Counts {
  /** What each Usage kind may cost, in one currency. */
  readonly price?: { readonly [Kind in keyof Counts]?: Money | undefined } | undefined;
}

/** Every Usage kind a Limit names, by its SDK name: Foxhound's `paidFetches` too. */
const USAGE_KINDS: Readonly<Record<string, string>> = {
  modelTokens: "model_tokens",
  modelCalls: "model_calls",
  browserTime: "browser_time",
  proxyBytes: "proxy_bytes",
  paidSolves: "paid_solves",
  requests: "requests",
  pageCrashes: "page_crashes",
  paidFetches: "paid_fetch",
};

/** Millionths in one unit of a currency. */
const MILLIONTHS = 1_000_000n;

const DECIMAL = /^(\d+)(?:\.(\d+))?$/;
const CURRENCY = /^[A-Z]{3}$/;

/**
 * The wire's list of `limits`: each count in the order of the Usage kinds,
 * then each price, in millionths of its currency; empty for none.
 *
 * @throws {SettingInvalidError} for a key that names no Usage kind, a count
 * that is no whole number the wire carries exactly, or a `Money` that does
 * not turn exactly into millionths.
 */
export function limitList(limits: Limits | undefined): RunLimit[] {
  if (limits === undefined) {
    return [];
  }
  const given: Readonly<Record<string, unknown>> = { ...limits };
  const counts: RunLimit[] = [];
  for (const [field, value] of Object.entries(given)) {
    if (field === "price" || value === undefined) {
      continue;
    }
    counts.push({ usage_kind: usageKind(`limits.${field}`, field), amount: count(field, value) });
  }
  const prices: RunLimit[] = [];
  const price: Readonly<Record<string, unknown>> = limits.price ?? {};
  for (const [field, value] of Object.entries(price)) {
    if (value === undefined) {
      continue;
    }
    const key = `limits.price.${field}`;
    const usage_kind = usageKind(key, field);
    const { currency, amount } = millionths(key, value);
    prices.push({ usage_kind, currency, amount });
  }
  return [...inKindOrder(counts), ...inKindOrder(prices)];
}

/** `limits`, in the order of `USAGE_KINDS`. */
function inKindOrder(limits: readonly RunLimit[]): RunLimit[] {
  const order = Object.values(USAGE_KINDS);
  return limits.toSorted((a, b) => order.indexOf(a.usage_kind) - order.indexOf(b.usage_kind));
}

/** The wire's Usage kind of the SDK name `field`. */
function usageKind(key: string, field: string): string {
  const kind = USAGE_KINDS[field];
  if (kind === undefined) {
    throw invalid(
      key,
      `\`${field}\` names no Usage kind`,
      `Name one of ${Object.keys(USAGE_KINDS).join(", ")}.`,
    );
  }
  return kind;
}

/** `value` as a count of the Usage kind `field`. */
function count(field: string, value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw invalid(
      `limits.${field}`,
      "a count is a whole number from 0 to 2^53 - 1, which the wire carries exactly",
      "Give a whole number of the Usage kind.",
    );
  }
  return value;
}

/** `value`, a `Money`, as millionths of its currency. */
function millionths(key: string, value: unknown): { currency: string; amount: number } {
  if (
    typeof value !== "object" ||
    value === null ||
    !("amount" in value) ||
    !("currency" in value)
  ) {
    throw invalid(
      key,
      "a price is a Money, { amount, currency }",
      'Give it as { amount: "0.50", currency: "USD" }.',
    );
  }
  const { amount, currency } = value;
  if (typeof amount !== "string") {
    throw invalid(
      key,
      "a Money amount is a decimal string, never a number, so that no float rounding reaches a Limit",
      `Give the amount as a string, such as "${String(amount)}".`,
    );
  }
  if (typeof currency !== "string" || !CURRENCY.test(currency)) {
    throw invalid(
      key,
      "a Money currency is an ISO 4217 code",
      "Give three capital letters, such as USD.",
    );
  }
  const parts = DECIMAL.exec(amount);
  const [, units, decimals = ""] = parts ?? [];
  if (units === undefined) {
    throw invalid(
      key,
      `\`${amount}\` is no decimal amount`,
      'Give digits with at most one decimal point, such as "0.50".',
    );
  }
  if (/[^0]/.test(decimals.slice(6))) {
    throw invalid(
      key,
      `\`${amount}\` is finer than a millionth of ${currency}`,
      "Give at most six decimals.",
    );
  }
  const total = BigInt(units) * MILLIONTHS + BigInt(decimals.slice(0, 6).padEnd(6, "0"));
  if (total > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw invalid(
      key,
      `\`${amount}\` is more millionths of ${currency} than the wire carries exactly`,
      "Give a smaller amount.",
    );
  }
  return { currency, amount: Number(total) };
}

function invalid(key: string, reason: string, fix: string): SettingInvalidError {
  return new SettingInvalidError(`${key}: ${reason}`, { fix, details: { key, reason } });
}
