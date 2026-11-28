// Field-level extraction quality against seeded Proving ground truth
// (ADR 0025, issue 185). Compares extracted records by key field (default `id`)
// against expected records and reports exact-match rate plus micro-averaged
// and per-field precision, recall and F1.

/** Field-level accuracy breakdown for one field name. */
export interface PerFieldQuality {
  readonly field: string;
  readonly expected: number;
  readonly extracted: number;
  readonly matched: number;
  readonly precision: number;
  readonly recall: number;
  readonly f1: number;
}

/** Field-level extraction quality report for a task or campaign. */
export interface ExtractionQualityReport {
  readonly records_expected: number;
  readonly records_extracted: number;
  readonly records_exact_match: number;
  readonly exact_match_rate: number;
  readonly field_precision: number;
  readonly field_recall: number;
  readonly field_f1: number;
  readonly per_field: readonly PerFieldQuality[];
}

function f1Of(precision: number, recall: number): number {
  return precision + recall === 0 ? 0 : (2 * precision * recall) / (precision + recall);
}

function normalizedValue(value: unknown): string {
  if (value === null || value === undefined) {
    return "";
  }
  if (typeof value === "string") {
    return value.trim();
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return JSON.stringify(value);
}

function isNonEmptyField(value: unknown): boolean {
  return normalizedValue(value).length > 0;
}

/**
 * Scores `actual` extracted records against `expected` seeded ground truth
 * records, aligning by `keyField` (default `"id"`).
 */
export function scoreExtractionQuality(input: {
  readonly actual: readonly Readonly<Record<string, unknown>>[];
  readonly expected: readonly Readonly<Record<string, unknown>>[];
  readonly keyField?: string;
}): ExtractionQualityReport {
  const keyField = input.keyField ?? "id";
  const actualByKey = new Map<string, Readonly<Record<string, unknown>>>();
  for (const rec of input.actual) {
    const key = normalizedValue(rec[keyField]);
    if (key.length > 0 && !actualByKey.has(key)) {
      actualByKey.set(key, rec);
    }
  }

  const fieldStats = new Map<string, { expected: number; extracted: number; matched: number }>();
  const touch = (field: string) => {
    let entry = fieldStats.get(field);
    if (entry === undefined) {
      entry = { expected: 0, extracted: 0, matched: 0 };
      fieldStats.set(field, entry);
    }
    return entry;
  };

  let exactMatches = 0;
  let totalExpectedFields = 0;
  let totalExtractedFields = 0;
  let totalMatchedFields = 0;

  const matchedKeys = new Set<string>();
  for (const exp of input.expected) {
    const key = normalizedValue(exp[keyField]);
    const act = actualByKey.get(key);
    if (act !== undefined) {
      matchedKeys.add(key);
    }

    const keys = new Set([...Object.keys(exp), ...(act === undefined ? [] : Object.keys(act))]);
    let recordExact = act !== undefined;

    for (const field of keys) {
      const expHas = isNonEmptyField(exp[field]);
      const actHas = act !== undefined && isNonEmptyField(act[field]);
      const stat = touch(field);
      if (expHas) {
        stat.expected += 1;
        totalExpectedFields += 1;
      }
      if (actHas) {
        stat.extracted += 1;
        totalExtractedFields += 1;
      }
      if (expHas && actHas && normalizedValue(exp[field]) === normalizedValue(act[field])) {
        stat.matched += 1;
        totalMatchedFields += 1;
      } else if (expHas || actHas) {
        recordExact = false;
      }
    }

    if (recordExact) {
      exactMatches += 1;
    }
  }

  // Extra records not in expected count against precision.
  for (const [key, act] of actualByKey.entries()) {
    if (matchedKeys.has(key)) {
      continue;
    }
    for (const [field, val] of Object.entries(act)) {
      if (isNonEmptyField(val)) {
        const stat = touch(field);
        stat.extracted += 1;
        totalExtractedFields += 1;
      }
    }
  }

  const precision = totalExtractedFields === 0 ? 0 : totalMatchedFields / totalExtractedFields;
  const recall = totalExpectedFields === 0 ? 0 : totalMatchedFields / totalExpectedFields;
  const exactMatchRate = input.expected.length === 0 ? 0 : exactMatches / input.expected.length;

  const perField: PerFieldQuality[] = [...fieldStats.entries()]
    .toSorted(([a], [b]) => a.localeCompare(b))
    .map(([field, s]) => {
      const p = s.extracted === 0 ? 0 : s.matched / s.extracted;
      const r = s.expected === 0 ? 0 : s.matched / s.expected;
      return {
        field,
        expected: s.expected,
        extracted: s.extracted,
        matched: s.matched,
        precision: p,
        recall: r,
        f1: f1Of(p, r),
      };
    });

  return {
    records_expected: input.expected.length,
    records_extracted: input.actual.length,
    records_exact_match: exactMatches,
    exact_match_rate: exactMatchRate,
    field_precision: precision,
    field_recall: recall,
    field_f1: f1Of(precision, recall),
    per_field: perField,
  };
}
