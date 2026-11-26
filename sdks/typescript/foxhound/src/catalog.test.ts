// Foxhound's catalog against this package (ADRs 0095 and 0099): each Error
// code, either Product's, has its class under its group in the classes
// Foxhound's Connection raises, and each event only Foxhound sends has its
// entry in the events registry, so the catalog gains none without its
// counterpart here. Python's `test_catalog.py` checks the same for the
// codes only Foxhound carries, the classes its package generates.
import { readFileSync } from "node:fs";
import { errorGroupClasses, type ErrorGroup } from "@goodfetch/collie-client";
import { describe, expect, test } from "vitest";
import { foxhoundEvents } from "./generated/events.ts";
import { foxhound } from "./product.ts";

interface CatalogEntry {
  readonly name: string;
  readonly product: "collie" | "foxhound";
}

interface CatalogCode extends CatalogEntry {
  readonly group: ErrorGroup;
  readonly retryable: boolean;
}

// oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the committed catalog file lists each Error code and event with the Product defining it.
const catalog = JSON.parse(
  readFileSync(
    new URL("../../../../crates/goodfetch-protocol/generated/foxhound.json", import.meta.url),
    "utf8",
  ),
) as { readonly error_codes: readonly CatalogCode[]; readonly events: readonly CatalogEntry[] };

describe("Foxhound's catalog", () => {
  // A code named like its group, such as `internal`, is raised as its
  // group's class (ADR 0099).
  test.for(catalog.error_codes)(
    "raises $name under its group, with its code, group and retryable flag",
    ({ name, group, retryable }) => {
      const known = foxhound.errorClasses[name];
      const error =
        known === undefined
          ? new errorGroupClasses[group]("failed", { code: name, group, retryable })
          : new known("failed");

      expect({
        isGroupClass: error instanceof errorGroupClasses[group],
        hasOwnClass: known !== undefined || name === group,
        code: error.code,
        group: error.group,
        retryable: error.retryable,
      }).toEqual({ isGroupClass: true, hasOwnClass: true, code: name, group, retryable });
    },
  );

  test("has an entry in the events registry for each event only Foxhound sends", () => {
    const foxhounds = catalog.events
      .filter(({ product }) => product === "foxhound")
      .map(({ name }) => name);

    expect(foxhounds).not.toEqual([]);
    expect(Object.keys(foxhoundEvents).toSorted()).toEqual(foxhounds.toSorted());
  });
});
