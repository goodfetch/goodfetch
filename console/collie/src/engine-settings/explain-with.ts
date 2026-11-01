// What Engine & settings explains with beside the engine's own layers, as
// `config explain --project <file> --set <key>=<value>` does (ADRs 0028 and
// 0083): a project configuration the person picks, whose `[settings]` table
// the Console reads in the browser and sends, as every Client sends the
// project configuration it read where it runs, and run values the person
// writes as TOML, one setting per line. The Console never writes either
// (ADR 0031). The run values are a run's own values, which a person edits
// row by row too (`withRunValue`).
import type { ConfigExplainParams, ProjectSettings } from "@goodfetch/collie-client/generated";
import { parse, stringify } from "smol-toml";

/** What `config.explain` takes beside the keys: the project configuration and run values given. */
export type ExplainWith = Pick<ConfigExplainParams, "project" | "settings">;

/** A project configuration the person picked: its file's name and text. */
export interface ProjectFile {
  readonly name: string;
  readonly text: string;
}

/** Why the Console sends nothing: the project configuration or the run values are no TOML. */
export type ExplainRefusal =
  | { readonly kind: "project"; readonly file: string; readonly reason: string }
  | { readonly kind: "run_values"; readonly reason: string };

/** The params, or why there are none. */
export type ExplainWithResult =
  | { readonly ok: true; readonly with: ExplainWith }
  | { readonly ok: false; readonly refusal: ExplainRefusal };

type Table = Record<string, unknown>;

function isTable(value: unknown): value is Table {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The TOML document `text`, or why it is none. */
function table(text: string): { readonly table: Table } | { readonly reason: string } {
  try {
    return { table: parse(text) };
  } catch (error) {
    return { reason: error instanceof Error ? error.message : String(error) };
  }
}

/** What to explain with `project`, when picked, and the run values `runValues`, when any. */
export function explainWith(
  project: ProjectFile | undefined,
  runValues: string,
): ExplainWithResult {
  const params: { project?: ProjectSettings; settings?: Table } = {};
  if (project !== undefined) {
    const read = table(project.text);
    if ("reason" in read) {
      return { ok: false, refusal: { kind: "project", file: project.name, reason: read.reason } };
    }
    const settings = read.table["settings"] ?? {};
    if (!isTable(settings)) {
      return {
        ok: false,
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- a reason, data the screen shows inside its own message.
        refusal: { kind: "project", file: project.name, reason: "its settings are no table" },
      };
    }
    params.project = { path: project.name, settings };
  }
  if (runValues.trim() !== "") {
    const read = table(runValues);
    if ("reason" in read) {
      return { ok: false, refusal: { kind: "run_values", reason: read.reason } };
    }
    params.settings = read.table;
  }
  return { ok: true, with: params };
}

/** `runValues` with `key` set to `value`, nested by key as TOML nests it, or removed for `null`; or why `runValues` is no TOML. */
export function withRunValue(
  runValues: string,
  key: string,
  value: unknown,
):
  | { readonly ok: true; readonly runValues: string }
  | { readonly ok: false; readonly refusal: ExplainRefusal } {
  const read = runValues.trim() === "" ? { table: {} } : table(runValues);
  if ("reason" in read) {
    return { ok: false, refusal: { kind: "run_values", reason: read.reason } };
  }
  const root: Table = read.table;
  const segments = key.split(".");
  const last = segments.pop() ?? key;
  let current = root;
  for (const segment of segments) {
    const next = current[segment];
    if (isTable(next)) {
      current = next;
    } else {
      const created: Table = {};
      current[segment] = created;
      current = created;
    }
  }
  if (value === null) {
    delete current[last];
  } else {
    current[last] = value;
  }
  return { ok: true, runValues: stringify(root) };
}
