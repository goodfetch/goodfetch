// A suite's declared case list (ADRs 0033 and 0072), such as
// `conformance/clients/cases.toml`: its
// name and version, the capabilities it declares and each one's cases with
// the Features each covers. What a suite declares and did not reach stays
// `unproved` in its result document. A case's other fields, such as the
// Client conformance cases' `surfaces` and `modes`, are their readers'.
import { readFileSync } from "node:fs";
import { parse } from "smol-toml";
import { TestSupportError } from "./errors.ts";
import type { Suite } from "./generated/result-document.ts";

/** One case a suite declares. */
export interface DeclaredCase {
  /** Its ID, unique in the suite. */
  readonly id: string;
  /** The Features it proves. */
  readonly covers: readonly string[];
}

/** One capability a suite declares. */
export interface DeclaredCapability {
  /** Its name, unique in the suite. */
  readonly name: string;
  /** Its cases, every one of which must pass for it to be `proved`. */
  readonly cases: readonly DeclaredCase[];
}

/** A suite's declared case list. */
export interface CaseList {
  readonly suite: Suite;
  /** Each capability, in the order its document lists them. */
  readonly capabilities: readonly DeclaredCapability[];
}

/**
 * The case list of `text`; `case_list_invalid` when it is no TOML, has no
 * `[suite]` with a name and version, or a capability has no name or a case
 * no ID or `covers`.
 */
export function parseCaseList(text: string): CaseList {
  let document: Record<string, unknown>;
  try {
    document = parse(text);
  } catch (error) {
    throw new TestSupportError("case_list_invalid", "the case list is no TOML", { cause: error });
  }
  const suite = document["suite"];
  if (
    !isTable(suite) ||
    typeof suite["name"] !== "string" ||
    typeof suite["version"] !== "string"
  ) {
    throw new TestSupportError(
      "case_list_invalid",
      "the case list has no [suite] name and version",
    );
  }
  const capabilities = tables(document["capability"], "capability").map((capability, index) => {
    const name = capability["name"];
    if (typeof name !== "string") {
      throw new TestSupportError(
        "case_list_invalid",
        `capability ${index + 1} of the case list has no name`,
      );
    }
    const cases = tables(capability["case"], `${name}'s case`).map((declared, at) => {
      const id = declared["id"];
      const covers = declared["covers"];
      if (typeof id !== "string") {
        throw new TestSupportError(
          "case_list_invalid",
          `case ${at + 1} of capability ${name} has no ID`,
        );
      }
      if (!Array.isArray(covers) || !covers.every((one) => typeof one === "string")) {
        throw new TestSupportError(
          "case_list_invalid",
          `case ${id} of capability ${name} has no \`covers\` list of Feature IDs`,
        );
      }
      return { id, covers };
    });
    return { name, cases };
  });
  return { suite: { name: suite["name"], version: suite["version"] }, capabilities };
}

/** The case list at `path`; `case_list_invalid` when it cannot be read or parsed. */
export function readCaseList(path: string): CaseList {
  let text: string;
  try {
    text = readFileSync(path, "utf8");
  } catch (error) {
    throw new TestSupportError("case_list_invalid", `the case list ${path} cannot be read`, {
      cause: error,
    });
  }
  return parseCaseList(text);
}

function tables(value: unknown, what: string): Record<string, unknown>[] {
  if (value === undefined) {
    return [];
  }
  if (!Array.isArray(value) || !value.every(isTable)) {
    throw new TestSupportError(
      "case_list_invalid",
      `the case list's ${what} is no array of tables`,
    );
  }
  return value;
}

function isTable(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
