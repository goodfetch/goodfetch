// The case log of Client conformance cases (ADR 0069): one JSON Lines entry
// per time a Vitest test ran a case, with no outcome field, so only
// `goodfetch-conformance`'s writer derives outcomes from it. A test carries
// its case as the tag `client_case/<id>` and its mode as the tag `attached`
// or `daemon`, both declared from the case list, so `strictTags` refuses an
// unknown case; `xtask clients check` reads the same tags from Vitest's
// listing. Each entry is checked against the bundle's `case_log_entry`
// schema before it is written.
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import type { ValidateFunction } from "ajv/dist/2020.js";
import type { TestTagDefinition } from "vitest";
import type { Reporter, TestCase } from "vitest/node";
import { parseCaseList } from "./case-list.ts";
import { TestSupportError } from "./errors.ts";
import type {
  CaseCheck,
  CaseLogEntry,
  Evidence,
  OracleAnswer,
  Platform,
} from "./generated/result-document.ts";
import { ajv } from "./schemas.ts";

/** The prefix of the tag that carries a case's ID. */
export const CLIENT_CASE_TAG = "client_case/";

/** The modes a TypeScript test runs a case in (ADR 0069). */
export const CLIENT_CASE_MODES: readonly string[] = ["attached", "daemon"];

/** A case of the Client conformance case list, as far as the runners read it. */
export interface ClientCase {
  readonly id: string;
  /** The capability it proves. */
  readonly capability: string;
}

/**
 * The cases of a case list's text, such as `conformance/clients/cases.toml`'s,
 * in its order; `case_list_invalid` when `parseCaseList` refuses it. Its
 * other fields are `xtask clients check`'s and the writer's.
 */
export function parseClientCases(text: string): ClientCase[] {
  return parseCaseList(text).capabilities.flatMap(({ name, cases }) =>
    cases.map(({ id }) => ({ id, capability: name })),
  );
}

/** The cases of the case list at `path`; none when there is no list yet. */
export function readClientCases(path: string): ClientCase[] {
  if (!existsSync(path)) {
    return [];
  }
  return parseClientCases(readFileSync(path, "utf8"));
}

/** The tags a case's test carries, for the Vitest configuration's `tags`: one per case, then one per mode. */
export function clientCaseTags(cases: readonly ClientCase[]): TestTagDefinition[] {
  return [
    ...cases.map((declared) => ({
      name: `${CLIENT_CASE_TAG}${declared.id}`,
      description: `Runs the Client conformance case ${declared.id} (${declared.capability}).`,
    })),
    ...CLIENT_CASE_MODES.map((mode) => ({
      name: mode,
      description: `Runs a Client conformance case in the ${mode} mode.`,
    })),
  ];
}

const bundle: { $defs: object } = JSON.parse(
  readFileSync(
    new URL("../../../crates/goodfetch-protocol/generated/bundle.json", import.meta.url),
    "utf8",
  ),
);
const isCaseLogEntry: ValidateFunction<CaseLogEntry> = ajv.compile<CaseLogEntry>({
  $ref: "#/$defs/CaseLogEntry",
  $defs: bundle.$defs,
});

/** `value` as a case log entry; `case_log_entry_invalid` when it fails the bundle's schema, such as by setting an outcome. */
export function checkedCaseLogEntry(value: unknown): CaseLogEntry {
  if (!isCaseLogEntry(value)) {
    throw new TestSupportError(
      "case_log_entry_invalid",
      `the case log entry fails the bundle's schema: ${ajv.errorsText(isCaseLogEntry.errors)}`,
    );
  }
  return value;
}

/** This host's operating system and architecture as Rust's `std::env::consts` names them, which every result document uses. */
export function hostPlatform(): Platform {
  return rustPlatform(process.platform, process.arch);
}

/** Node's `process.platform` and `process.arch` as Rust's `std::env::consts` names them: `darwin` is `macos`, `x64` is `x86_64`. */
export function rustPlatform(platform: string, arch: string): Platform {
  const os: Record<string, string> = { darwin: "macos", win32: "windows" };
  const cpu: Record<string, string> = { x64: "x86_64", arm64: "aarch64", ia32: "x86" };
  return { os: os[platform] ?? platform, arch: cpu[arch] ?? arch };
}

/** The runtime the cases run on, such as `node-24`. */
export function hostRuntime(): string {
  return `node-${process.versions.node.split(".")[0]}`;
}

/** The runtime a Playwright test runs on: its browser and the browser's major version, such as `chromium-141`. */
export function browserRuntime(browserName: string, version: string): string {
  return `${browserName}-${version.split(".")[0] ?? ""}`;
}

/** What a test recorded of the case it runs, which the reporter reads from the test's meta. */
export interface CaseRecording {
  readonly checks: CaseCheck[];
  readonly evidence: Required<Evidence>;
}

declare module "vitest" {
  interface TaskMeta {
    /** What the test recorded of its Client conformance case (ADR 0069). */
    clientCase?: CaseRecording;
  }
}

/**
 * Records a case's checks and evidence for its case log entry. Each check
 * runs an assertion and records whether it held; a check that fails still
 * fails the test.
 */
export class ClientCaseRecord {
  readonly #recording: CaseRecording;

  constructor(recording: CaseRecording) {
    this.#recording = recording;
  }

  /** Runs `assertion` as the check `name`, records whether it held, and rethrows its failure. */
  async check(name: string, assertion: () => void | Promise<void>): Promise<void> {
    try {
      await assertion();
    } catch (error) {
      this.#recording.checks.push({ name, passed: false });
      throw error;
    }
    this.#recording.checks.push({ name, passed: true });
  }

  /** Records a scripted message the case exchanged. */
  message(value: unknown): void {
    this.#recording.evidence.messages.push(value);
  }

  /** Records an oracle's answer the case received. */
  oracleAnswer(answer: OracleAnswer): void {
    this.#recording.evidence.oracle_answers.push(answer);
  }

  /** Records what the Proving ground observed on the server side. */
  observation(value: unknown): void {
    this.#recording.evidence.observations.push(value);
  }
}

/** A running test, as far as its case's record reads it. */
interface CaseTask {
  readonly name: string;
  readonly tags?: readonly string[] | undefined;
  readonly meta: { clientCase?: CaseRecording };
}

/** The record of the case `task` runs, kept in its meta; `client_case_untagged` when it carries no `client_case/` tag. */
export function clientCaseRecord(task: CaseTask): ClientCaseRecord {
  if (!(task.tags ?? []).some((tag) => tag.startsWith(CLIENT_CASE_TAG))) {
    throw new TestSupportError(
      "client_case_untagged",
      `the test "${task.name}" records a Client conformance case but carries no ${CLIENT_CASE_TAG}<id> tag`,
    );
  }
  task.meta.clientCase ??= {
    checks: [],
    evidence: { oracle_answers: [], observations: [], messages: [] },
  };
  return new ClientCaseRecord(task.meta.clientCase);
}

/** Where the reporter writes and which case list it reads. */
export interface CaseLogOptions {
  /** The case log's file, which each run starts empty. */
  readonly out: string;
  /** The case list, such as `conformance/clients/cases.toml`, which gives each case's capability. */
  readonly caseList: string;
}

/** The check every entry ends with: whether Vitest passed the test, so a test that fails outside its checks fails its case. */
const TEST_PASSED = "the test passed";

/**
 * The Vitest reporter that writes the case log: one entry per case a test
 * that ran carries, with the case's capability from the case list, the
 * test's mode tag, this host's platform and runtime, the test's recorded
 * checks followed by whether it passed, and its recorded evidence. A skipped
 * test writes nothing, so its case stays `not_reached`.
 */
export class CaseLogReporter implements Reporter {
  readonly #out: string;
  readonly #capabilities: ReadonlyMap<string, string>;

  constructor(options: CaseLogOptions) {
    this.#out = options.out;
    this.#capabilities = new Map(
      readClientCases(options.caseList).map((declared) => [declared.id, declared.capability]),
    );
  }

  onTestRunStart(): void {
    mkdirSync(dirname(this.#out), { recursive: true });
    writeFileSync(this.#out, "");
  }

  onTestCaseResult(testCase: TestCase): void {
    const cases = testCase.tags
      .filter((tag) => tag.startsWith(CLIENT_CASE_TAG))
      .map((tag) => tag.slice(CLIENT_CASE_TAG.length));
    const result = testCase.result();
    if (cases.length === 0 || result.state === "skipped" || result.state === "pending") {
      return;
    }
    const modes = testCase.tags.filter((tag) => CLIENT_CASE_MODES.includes(tag));
    const recording = testCase.meta().clientCase;
    const checks = [
      ...(recording?.checks ?? []),
      { name: TEST_PASSED, passed: result.state === "passed" },
    ];
    const evidence = Object.fromEntries(
      Object.entries(recording?.evidence ?? {}).filter(([, values]) => values.length > 0),
    );
    const lines = cases.map((id) => {
      const capability = this.#capabilities.get(id);
      if (capability === undefined) {
        throw new TestSupportError(
          "client_case_unknown",
          `the test "${testCase.fullName}" carries the case ${id}, which the case list does not declare`,
        );
      }
      const entry = checkedCaseLogEntry({
        case: id,
        capability,
        ...(modes.length === 1 ? { mode: modes[0] } : {}),
        platform: hostPlatform(),
        runtime: hostRuntime(),
        checks,
        evidence,
      });
      return `${JSON.stringify(entry)}\n`;
    });
    appendFileSync(this.#out, lines.join(""));
  }
}
