/**
 * Whether a Client's executable would be its major's pick once it ran
 * (ADR 0098), read from the list of installed executables in Goodfetch
 * home, `installed-executables.json`, as the engine's `installed::pick`
 * decides it: among the Products' executables of the line that count as
 * installed and can be started, Foxhound before Collie, then the newest
 * version. A refusal's `fix` names Foxhound only where it would not be.
 *
 * It differs from the engine's where ADR 0098's note of 2026-11-21 says:
 * `browser_cache.max_unused_age` is its default, the shared stores' lowest
 * minor is not read, a copy counts as present by its folder, and the cache
 * `GOODFETCH_BROWSERS_PATH` names is read even where it cannot be written.
 */
import { readFile, stat } from "node:fs/promises";
import { join } from "node:path";
import { isRecord } from "./framing.ts";
import type { Product } from "./generated/types.ts";

/** The Client whose executable is asked about: its Product and its Product version. */
export interface PickedClient {
  readonly product: Product;
  readonly version: string;
}

/** `browser_cache.max_unused_age`'s default, past which an executable gone from its path no longer counts. */
const MAX_UNUSED_AGE_MS = 30 * 24 * 60 * 60 * 1000;
const BROWSERS_PATH_VARIABLE = "GOODFETCH_BROWSERS_PATH";
const ARTIFACT_KINDS: ReadonlySet<unknown> = new Set([
  "browser",
  "driver",
  "executable_copy",
  "engine_executable",
  "clean_up_rules",
  "mouse_movements",
]);
const U64_MAX = 2n ** 64n - 1n;

/**
 * Whether `client`'s executable, recorded in `home`'s list as it would be
 * on running, would be its major's pick at `now`: no executable of its line
 * that counts as installed and can be started outranks it. A list that
 * cannot be read names none, as the engine reads it.
 */
export async function wouldBeThePick(
  client: PickedClient,
  home: string,
  env: Readonly<Record<string, string>>,
  now: Date,
): Promise<boolean> {
  const own = productVersion(client.version);
  if (own === undefined) {
    return true;
  }
  const cache = browserCache(home, env);
  for (const entry of await installed(home)) {
    if (
      entry.kind === "product_executable" &&
      sameLine(entry.version, own) &&
      outranks(entry, client.product, own) &&
      (await startable(entry, cache, now))
    ) {
      return false;
    }
  }
  return true;
}

/** One entry of the list, as the engine reads it. */
interface Entry {
  readonly kind: "product_executable" | "linked_program";
  readonly product: Product;
  readonly version: ProductVersion;
  readonly path: string;
  readonly size: number;
  readonly modifiedAt: Instant;
  readonly copy: ArtifactId | null;
  readonly lastSeen: Instant;
}

interface ArtifactId {
  readonly kind: string;
  readonly name: string;
  readonly build: string;
}

/** A point in time in RFC 3339: its whole seconds since the epoch, in milliseconds, and its fraction of a second. */
interface Instant {
  readonly seconds: number;
  readonly fraction: string;
}

/**
 * The entries of `home`'s list it can read: none when the list is missing,
 * is not a JSON object or is of another major than 0 (its rule is
 * FrozenMajor), and only the entries in the shape the engine reads.
 */
async function installed(home: string): Promise<readonly Entry[]> {
  let document: unknown;
  try {
    document = JSON.parse(await readFile(join(home, "installed-executables.json"), "utf8"));
  } catch {
    return [];
  }
  if (!isRecord(document) || formatMajor(document["format_version"]) !== 0) {
    return [];
  }
  const executables = document["executables"];
  return Array.isArray(executables) ? executables.flatMap((value) => entryOf(value) ?? []) : [];
}

/** The major of a format version `major.minor`, each a canonical number up to 65535. */
function formatMajor(value: unknown): number | undefined {
  if (typeof value !== "string") {
    return undefined;
  }
  const match = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.exec(value);
  if (match === null) {
    return undefined;
  }
  const [major, minor] = [Number(match[1]), Number(match[2])];
  return major <= 0xffff && minor <= 0xffff ? major : undefined;
}

function entryOf(value: unknown): Entry | undefined {
  if (!isRecord(value)) {
    return undefined;
  }
  const { kind, product, path, size, pins } = value;
  const version = productVersion(value["product_version"]);
  const modifiedAt = instant(value["modified_at"]);
  const lastSeen = instant(value["last_seen"]);
  const copy = value["copy"] ?? null;
  if (
    (kind !== "product_executable" && kind !== "linked_program") ||
    (product !== "collie" && product !== "foxhound") ||
    version === undefined ||
    typeof path !== "string" ||
    typeof size !== "number" ||
    !Number.isSafeInteger(size) ||
    size < 0 ||
    modifiedAt === undefined ||
    lastSeen === undefined ||
    !Array.isArray(pins) ||
    !pins.every(isArtifactId) ||
    (copy !== null && !isArtifactId(copy))
  ) {
    return undefined;
  }
  return { kind, product, version, path, size, modifiedAt, copy, lastSeen };
}

function isArtifactId(value: unknown): value is ArtifactId {
  return (
    isRecord(value) &&
    ARTIFACT_KINDS.has(value["kind"]) &&
    typeof value["name"] === "string" &&
    typeof value["build"] === "string"
  );
}

/** `value` read as RFC 3339; `undefined` when it is not. */
function instant(value: unknown): Instant | undefined {
  if (typeof value !== "string") {
    return undefined;
  }
  const match =
    /^(\d{4}-\d{2}-\d{2}[Tt ]\d{2}:\d{2}:\d{2})(?:\.(\d+))?([Zz]|[+-]\d{2}:\d{2})$/.exec(value);
  if (match === null) {
    return undefined;
  }
  const seconds = Date.parse(`${match[1]?.replace(/[t ]/, "T")}${match[3]?.toUpperCase()}`);
  return Number.isNaN(seconds)
    ? undefined
    : { seconds, fraction: (match[2] ?? "").replace(/0+$/, "") };
}

/** `at` in milliseconds since the epoch. */
function millisecondsOf(at: Instant): number {
  return at.seconds + Number(`0.${at.fraction || "0"}`) * 1000;
}

/** Whether `entry`'s Product and version rank above `product` at `version`: Foxhound before Collie, then the newest version. */
function outranks(entry: Entry, product: Product, version: ProductVersion): boolean {
  const byProduct = productRank(entry.product) - productRank(product);
  return byProduct === 0 ? compareVersions(entry.version, version) > 0 : byProduct > 0;
}

/** Foxhound, which contains Collie, above Collie. */
function productRank(product: Product): number {
  return product === "foxhound" ? 1 : 0;
}

/**
 * Whether `entry` counts as installed at `now` and can be started: its file
 * is still at its path at its size and modification time, or it was seen
 * within `browser_cache.max_unused_age` and its versioned copy is in the
 * Browser cache at `cache`.
 */
async function startable(entry: Entry, cache: string, now: Date): Promise<boolean> {
  if (await stillAtPath(entry)) {
    return true;
  }
  const elapsed = now.getTime() - millisecondsOf(entry.lastSeen);
  return (
    elapsed < MAX_UNUSED_AGE_MS && entry.copy !== null && (await copyPresent(cache, entry.copy))
  );
}

/** Whether the file at `entry`'s path has its size and its modification time in whole seconds. */
async function stillAtPath(entry: Entry): Promise<boolean> {
  try {
    const found = await stat(entry.path);
    return (
      found.isFile() &&
      found.size === entry.size &&
      entry.modifiedAt.fraction === "" &&
      Math.floor(found.mtimeMs / 1000) * 1000 === entry.modifiedAt.seconds
    );
  } catch {
    return false;
  }
}

/** Whether the copy `id` has its entry and its files in the Browser cache at `cache`. */
async function copyPresent(cache: string, id: ArtifactId): Promise<boolean> {
  const folder = join(cache, id.kind, id.name, id.build);
  try {
    const [entry, files] = await Promise.all([
      stat(join(folder, "entry.json")),
      stat(join(folder, "files")),
    ]);
    return entry.isFile() && files.isDirectory();
  } catch {
    return false;
  }
}

/** The Browser cache: the one `GOODFETCH_BROWSERS_PATH` names, else `browsers` in Goodfetch home. */
function browserCache(home: string, env: Readonly<Record<string, string>>): string {
  const named = env[BROWSERS_PATH_VARIABLE];
  return named === undefined || named === "" ? join(home, "browsers") : named;
}

/** A Product version `major.minor.patch` with an optional pre-release (ADR 0090), as the engine reads it. */
interface ProductVersion {
  readonly text: string;
  readonly major: bigint;
  readonly minor: bigint;
  readonly patch: bigint;
  readonly preRelease: readonly string[] | null;
}

function productVersion(value: unknown): ProductVersion | undefined {
  if (typeof value !== "string") {
    return undefined;
  }
  const match =
    /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(
      value,
    );
  if (match === null) {
    return undefined;
  }
  const [major, minor, patch] = [
    BigInt(match[1] ?? ""),
    BigInt(match[2] ?? ""),
    BigInt(match[3] ?? ""),
  ];
  if (major > U64_MAX || minor > U64_MAX) {
    return undefined;
  }
  return { text: value, major, minor, patch, preRelease: match[4]?.split(".") ?? null };
}

/** Whether `left` and `right` are of one line: one major, or one `0.y` minor during previews (ADR 0030). */
function sameLine(left: ProductVersion, right: ProductVersion): boolean {
  return left.major === right.major && (left.major !== 0n || left.minor === right.minor);
}

/**
 * Semantic Versioning's precedence, as the engine orders Product versions:
 * the numbers, then a release above its pre-releases, compared identifier
 * by identifier; then the text, so only one version equals another.
 */
function compareVersions(left: ProductVersion, right: ProductVersion): number {
  return (
    compareBig(left.major, right.major) ||
    compareBig(left.minor, right.minor) ||
    compareBig(left.patch, right.patch) ||
    comparePreReleases(left.preRelease, right.preRelease) ||
    compareText(left.text, right.text)
  );
}

function comparePreReleases(
  left: readonly string[] | null,
  right: readonly string[] | null,
): number {
  if (left === null || right === null) {
    return (left === null ? 1 : 0) - (right === null ? 1 : 0);
  }
  for (let index = 0; index < Math.min(left.length, right.length); index += 1) {
    const order = compareIdentifiers(left[index] ?? "", right[index] ?? "");
    if (order !== 0) {
      return order;
    }
  }
  return left.length - right.length;
}

/** Numbers numerically and below words, words by their bytes. */
function compareIdentifiers(left: string, right: string): number {
  const [leftNumber, rightNumber] = [numberOf(left), numberOf(right)];
  if (leftNumber !== undefined && rightNumber !== undefined) {
    return compareBig(leftNumber, rightNumber);
  }
  if (leftNumber !== undefined || rightNumber !== undefined) {
    return leftNumber === undefined ? 1 : -1;
  }
  return compareText(left, right);
}

/** An identifier of digits alone that fits 64 bits, as the engine reads a number. */
function numberOf(identifier: string): bigint | undefined {
  if (!/^[0-9]+$/.test(identifier)) {
    return undefined;
  }
  const number = BigInt(identifier);
  return number <= U64_MAX ? number : undefined;
}

function compareBig(left: bigint, right: bigint): number {
  return left === right ? 0 : left < right ? -1 : 1;
}

/** ASCII text by its bytes. */
function compareText(left: string, right: string): number {
  return left === right ? 0 : left < right ? -1 : 1;
}
