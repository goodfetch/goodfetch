// The project file `connect()` reads (ADR 0083): the `goodfetch.toml` found
// in the working directory or the nearest folder above it, the one the
// program names, or none, read once at `connect()` and refused there before
// any engine is reached, as the engine reads a project file: its `version`
// under ADR 0030's default rule, its `[settings]` table, and nothing else.
// The file is carried by its absolute path, so the engine, or a later read
// of the Run it starts, finds the same file.
import { readFile, stat } from "node:fs/promises";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { parse, TomlDate, type TomlTable, type TomlValue } from "smol-toml";
import { isRecord } from "./framing.ts";
import { PROJECT_FILE, PROJECT_FORMAT, PROJECT_FORMAT_VERSION } from "./generated/connection.ts";
import {
  FormatVersionUnsupportedError,
  SettingInvalidError,
  type ProjectSettings,
} from "./generated/index.ts";

/** The project file a connection read: where it is, its settings, and the `timeout.connect` it sets. */
export interface ProjectFile {
  /** Its absolute path and its settings, nested as the file nests them, as `run.start` and `config.explain` carry them. */
  readonly carried: ProjectSettings;
  /** The `timeout.connect` it sets, in milliseconds, already checked; none when it sets none. */
  readonly connectTimeout: number | undefined;
}

/** The layer a project file's values and refusals are at. */
const LAYER = "project_configuration";
/** The key that sets how long opening a Connection waits for `hello`'s answer. */
const TIMEOUT_CONNECT = "timeout.connect";
/** A file's top-level key naming the lowest minor that can use it (ADR 0098). */
const LOWEST_MINOR = "lowest_minor";
/** The file's one table. */
const SETTINGS = "settings";
/** What a program does in place of the project file refused. */
const PROJECT_FIX = "name another project file with `project`, or read none with `project: null`";

/**
 * The project file `project` chooses, read: the one found upward from
 * `workingDirectory()` when left out, the file it names, a relative one from
 * the working directory, or none for `null`.
 *
 * @throws {SettingInvalidError} at `project_configuration` when the working
 * directory cannot be read, for a named file that is not there, for a file
 * that does not read as a project file, and for a `timeout.connect` that is
 * no whole number of 1 ms or more.
 * @throws {FormatVersionUnsupportedError} for a file of a format version this
 * Goodfetch cannot read.
 */
export async function readProject(
  project: string | null | undefined,
  workingDirectory: () => string,
): Promise<ProjectFile | undefined> {
  if (project === null) {
    return undefined;
  }
  let folder: string;
  try {
    folder = workingDirectory();
  } catch (error) {
    throw noWorkingDirectory(error);
  }
  const file = project === undefined ? await findProject(folder) : absolute(folder, project);
  if (file === undefined) {
    return undefined;
  }
  const settings = projectSettings(file, await read(file));
  return {
    carried: { path: file, settings: jsonTable(file, settings) },
    connectTimeout: connectTimeoutOf(file, nested(settings, TIMEOUT_CONNECT)),
  };
}

/**
 * `project` made absolute from `folder`, as the Rust SDK's
 * `std::path::absolute` makes it, so that every SDK reports the same path:
 * on POSIX `.` components and repeated separators dropped and each `..`
 * kept for the file system to resolve past a symbolic link; on Windows
 * resolved whole, as `GetFullPathNameW` resolves it there.
 */
function absolute(folder: string, project: string): string {
  if (process.platform === "win32") {
    return resolve(folder, project);
  }
  const parts = (isAbsolute(project) ? project : `${folder}/${project}`)
    .split("/")
    .filter((part) => part !== "" && part !== ".");
  return `/${parts.join("/")}`;
}

/** The project file in `folder` or the nearest folder above it. */
async function findProject(folder: string): Promise<string | undefined> {
  for (let current = folder; ; current = dirname(current)) {
    const file = join(current, PROJECT_FILE);
    if (await isFile(file)) {
      return file;
    }
    if (dirname(current) === current) {
      return undefined;
    }
  }
}

async function isFile(path: string): Promise<boolean> {
  try {
    return (await stat(path)).isFile();
  } catch {
    return false;
  }
}

/** The text of the project file at `file`. */
async function read(file: string): Promise<string> {
  try {
    return await readFile(file, "utf8");
  } catch (error) {
    if (isRecord(error) && error["code"] === "ENOENT") {
      const reason = "the file is not there";
      throw new SettingInvalidError(`The project file ${file} cannot be read: ${reason}.`, {
        fix: `Create it, or ${PROJECT_FIX}.`,
        details: { key: file, layer: LAYER, reason },
        cause: error,
      });
    }
    throw invalidFile(file, error instanceof Error ? error.message : String(error), error);
  }
}

/**
 * The `[settings]` of the project file at `file`, whose text is `text`, after
 * checking its `version` and refusing any other top-level entry, as the
 * engine's own read does.
 */
function projectSettings(file: string, text: string): TomlTable {
  let parsed: TomlTable;
  try {
    // Every integer is read as a `bigint`, so a float such as `1.0`, which
    // the engine never reads as a whole number, stays apart from `1`.
    parsed = parse(text, { integersAsBigInt: true });
  } catch (error) {
    throw invalidFile(file, error instanceof Error ? error.message : String(error), error);
  }
  const { version, [LOWEST_MINOR]: lowestMinor, [SETTINGS]: settings, ...others } = parsed;
  if (typeof version !== "string") {
    throw invalidFile(file, "it names no `version`");
  }
  const found = versionOf(version);
  if (found === undefined) {
    throw invalidFile(file, `\`${version}\` is no \`major.minor\` version`);
  }
  if (lowestMinor !== undefined && !isMinor(lowestMinor)) {
    throw invalidFile(file, `\`${LOWEST_MINOR}\` is no minor`);
  }
  const supported = versionOf(PROJECT_FORMAT_VERSION);
  if (supported === undefined) {
    throw new TypeError("invariant: the generated project format version is `major.minor`");
  }
  const compatibility = compatibilityOf(supported, found);
  if (compatibility !== "compatible") {
    throw formatRefused(file, version, compatibility);
  }
  if (settings !== undefined && !isTable(settings)) {
    throw invalidFile(file, `\`${SETTINGS}\` is not a table`);
  }
  const other = Object.keys(others)[0];
  if (other !== undefined) {
    throw invalidFile(file, `\`${other}\` is no part of this file; settings go in \`[settings]\``);
  }
  return { ...settings };
}

function isTable(value: TomlValue): value is TomlTable {
  return isRecord(value) && !(value instanceof TomlDate);
}

/** A version `major.minor`, as the engine reads one: two canonical numbers of at most 65535. */
interface Version {
  readonly major: number;
  readonly minor: number;
}

function versionOf(text: string): Version | undefined {
  const match = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.exec(text);
  if (match === null) {
    return undefined;
  }
  const [major, minor] = [Number(match[1]), Number(match[2])];
  return major <= 0xffff && minor <= 0xffff ? { major, minor } : undefined;
}

function isMinor(value: TomlValue): boolean {
  return typeof value === "bigint" && value >= 0n && value <= 0xffffn;
}

/** What a reader of `reader` makes of a file of `found` under ADR 0030's default rule. */
type Compatibility = "compatible" | "newer_minor" | "newer_major" | "older_major";

/**
 * ADR 0030's default rule: a file of an older line needs `upgrade-files`, a
 * newer line or a newer minor of the same line a newer Goodfetch. During
 * `0.y` previews each `0.y` is a line of its own, whose only minor is 0.
 */
function compatibilityOf(reader: Version, found: Version): Compatibility {
  const [readerLine, readerMinor] = lineOf(reader);
  const [foundLine, foundMinor] = lineOf(found);
  const order = foundLine[0] - readerLine[0] || foundLine[1] - readerLine[1];
  if (order < 0) {
    return "older_major";
  }
  if (order > 0) {
    return "newer_major";
  }
  return foundMinor > readerMinor ? "newer_minor" : "compatible";
}

/** A version's line and its minor in it. */
function lineOf({ major, minor }: Version): readonly [readonly [number, number], number] {
  return major === 0 ? [[0, minor], 0] : [[major, 0], minor];
}

/** The `format_version_unsupported` of the project file at `file`, of the version `found`. */
function formatRefused(
  file: string,
  found: string,
  compatibility: Exclude<Compatibility, "compatible">,
): FormatVersionUnsupportedError {
  const [message, fix] =
    compatibility === "older_major"
      ? [
          `this ${PROJECT_FORMAT} file is ${found}, of an older major; run \`upgrade-files\` to upgrade it`,
          "Run `upgrade-files`, then try again.",
        ]
      : [
          `this ${PROJECT_FORMAT} file is ${found}, newer than this engine reads; upgrade Goodfetch to read it`,
          `Upgrade Goodfetch to a version that reads ${PROJECT_FORMAT} ${found}.`,
        ];
  return new FormatVersionUnsupportedError(`The project file ${file} cannot be read: ${message}`, {
    fix,
    details: { format: PROJECT_FORMAT, found, supported: PROJECT_FORMAT_VERSION, fix },
  });
}

/** The `setting_invalid` of the project file at `file`, which does not read for `reason`. */
function invalidFile(file: string, reason: string, cause?: unknown): SettingInvalidError {
  return new SettingInvalidError(`The project file ${file} cannot be read: ${reason}.`, {
    fix: `Fix the project file at ${file}, or ${PROJECT_FIX}.`,
    details: { key: file, layer: LAYER, reason },
    cause,
  });
}

/** The `setting_invalid` when the working directory, which the project file is found or named from, cannot be read. */
function noWorkingDirectory(error: unknown): SettingInvalidError {
  const reason = `the working directory cannot be read (${error instanceof Error ? error.message : String(error)})`;
  return new SettingInvalidError(`No project file can be found: ${reason}.`, {
    fix: "Name the project file by its absolute path with `project`, or read none with `project: null`.",
    details: { key: "project", layer: LAYER, reason },
    cause: error,
  });
}

/** The value of `key` in `settings`, written whole as one dotted key, or nested as a file nests it. */
function nested(settings: TomlTable, key: string): TomlValue | undefined {
  if (Object.hasOwn(settings, key)) {
    return settings[key];
  }
  let value: TomlValue | undefined = settings;
  for (const segment of key.split(".")) {
    if (value === undefined || !isTable(value) || !Object.hasOwn(value, segment)) {
      return undefined;
    }
    value = value[segment];
  }
  return value;
}

/**
 * The project file's `timeout.connect`, `value`, checked as the engine checks
 * it with the rest of the file.
 *
 * @throws {SettingInvalidError} for a value that is no whole number of 1 ms or more.
 */
function connectTimeoutOf(file: string, value: TomlValue | undefined): number | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (typeof value === "bigint" && value >= 1n && value <= BigInt(Number.MAX_SAFE_INTEGER)) {
    return Number(value);
  }
  const reason =
    value === 0n
      ? "a timeout of 0 ms would end the opening at once"
      : "it is no whole number of milliseconds";
  const fix = `set \`${TIMEOUT_CONNECT}\` in the project file to at least 1 ms, or remove it for 30 s`;
  const shown = jsonValue(file, value);
  throw new SettingInvalidError(
    `The project file's \`${TIMEOUT_CONNECT}\`, ${JSON.stringify(shown)}: ${reason}; ${fix}.`,
    {
      fix,
      limit: { key: TIMEOUT_CONNECT, value: shown, layer: LAYER },
      details: { key: TIMEOUT_CONNECT, layer: LAYER, reason },
    },
  );
}

/**
 * The project file at `file`'s TOML table, as JSON: an integer as a number,
 * a float JSON cannot hold as `null`, as the engine turns them, and a date
 * as its ISO 8601 text, which, unlike the engine's text of it, always names
 * its seconds and their milliseconds.
 *
 * @throws {SettingInvalidError} for an integer past what a number holds
 * exactly, which is refused rather than rounded.
 */
function jsonTable(file: string, table: TomlTable): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(table).map(([key, value]) => [key, jsonValue(file, value)]),
  );
}

function jsonValue(file: string, value: TomlValue): unknown {
  if (typeof value === "bigint") {
    if (value < BigInt(Number.MIN_SAFE_INTEGER) || value > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw invalidFile(file, `the integer \`${String(value)}\` cannot be carried exactly`);
    }
    return Number(value);
  }
  if (value instanceof TomlDate) {
    return value.toISOString();
  }
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : null;
  }
  if (Array.isArray(value)) {
    return value.map((item) => jsonValue(file, item));
  }
  if (typeof value === "object") {
    return jsonTable(file, value);
  }
  return value;
}
