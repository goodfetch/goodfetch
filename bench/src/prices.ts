// Versioned price tables for the benchmark cost model (ADR 0025): host hourly
// rates, model token rates, proxy rates, CAPTCHA solver rates and hosted
// browser per-minute rates, each with its source and date.
import { readFileSync } from "node:fs";
import { parserOf } from "./schema.ts";

/** Public hourly rate for a dedicated or rented machine. */
export interface HostPrice {
  readonly hourly_usd: number;
  readonly source: string;
  readonly date: string;
  readonly description: string;
}

/** List token rates for a language model. */
export interface ModelPrice {
  readonly input_per_1m_usd: number;
  readonly output_per_1m_usd: number;
  readonly source: string;
  readonly date: string;
}

/** List rate for a proxy tier. */
export interface ProxyPrice {
  readonly per_gb_usd: number;
  readonly per_request_usd?: number;
  readonly source: string;
  readonly date: string;
}

/** List rate for a CAPTCHA solver kind. */
export interface CaptchaPrice {
  readonly per_1k_solves_usd: number;
  readonly source: string;
  readonly date: string;
}

/** List rate for a hosted browser vendor per browser-minute. */
export interface HostedBrowserPrice {
  readonly per_browser_minute_usd: number;
  readonly source: string;
  readonly date: string;
}

/** One versioned price table with provenance (source and date). */
export interface PriceTable {
  readonly version: number;
  readonly effective_date: string;
  readonly source: string;
  readonly hosts: Readonly<Record<string, HostPrice>>;
  readonly models: Readonly<Record<string, ModelPrice>>;
  readonly proxies: Readonly<Record<string, ProxyPrice>>;
  readonly captchas: Readonly<Record<string, CaptchaPrice>>;
  readonly hosted_browsers: Readonly<Record<string, HostedBrowserPrice>>;
}

const isoDate = { type: "string", pattern: "^\\d{4}-\\d{2}-\\d{2}$" };
const nonNegative = { type: "number", minimum: 0 };
const nonEmpty = { type: "string", minLength: 1 };

const priceTableParser = parserOf<PriceTable>({
  type: "object",
  additionalProperties: false,
  required: [
    "version",
    "effective_date",
    "source",
    "hosts",
    "models",
    "proxies",
    "captchas",
    "hosted_browsers",
  ],
  properties: {
    version: { type: "integer", minimum: 1 },
    effective_date: isoDate,
    source: nonEmpty,
    hosts: {
      type: "object",
      additionalProperties: {
        type: "object",
        additionalProperties: false,
        required: ["hourly_usd", "source", "date", "description"],
        properties: {
          hourly_usd: nonNegative,
          source: nonEmpty,
          date: isoDate,
          description: nonEmpty,
        },
      },
    },
    models: {
      type: "object",
      additionalProperties: {
        type: "object",
        additionalProperties: false,
        required: ["input_per_1m_usd", "output_per_1m_usd", "source", "date"],
        properties: {
          input_per_1m_usd: nonNegative,
          output_per_1m_usd: nonNegative,
          source: nonEmpty,
          date: isoDate,
        },
      },
    },
    proxies: {
      type: "object",
      additionalProperties: {
        type: "object",
        additionalProperties: false,
        required: ["per_gb_usd", "source", "date"],
        properties: {
          per_gb_usd: nonNegative,
          per_request_usd: nonNegative,
          source: nonEmpty,
          date: isoDate,
        },
      },
    },
    captchas: {
      type: "object",
      additionalProperties: {
        type: "object",
        additionalProperties: false,
        required: ["per_1k_solves_usd", "source", "date"],
        properties: {
          per_1k_solves_usd: nonNegative,
          source: nonEmpty,
          date: isoDate,
        },
      },
    },
    hosted_browsers: {
      type: "object",
      additionalProperties: {
        type: "object",
        additionalProperties: false,
        required: ["per_browser_minute_usd", "source", "date"],
        properties: {
          per_browser_minute_usd: nonNegative,
          source: nonEmpty,
          date: isoDate,
        },
      },
    },
  },
});

/** Validates an untrusted price table document at the boundary. */
export function parsePriceTable(raw: unknown, label = "price table"): PriceTable {
  return priceTableParser.parse(raw, label);
}

/** Loads the versioned price table from `filePath` (defaults to `bench/prices/v1.json`). */
export function loadPriceTable(filePath?: string): PriceTable {
  const target =
    filePath === undefined ? new URL("../prices/v1.json", import.meta.url).pathname : filePath;
  const raw: unknown = JSON.parse(readFileSync(target, "utf8"));
  return parsePriceTable(raw, target);
}
