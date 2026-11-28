// Nightly Live check `adapters-latest` (ADRs 0064 and 0070; issue 1382).
//
// Reads every adapter's declared peer range from `adapters/*/*/package.json`
// and `adapters/*/*/pyproject.toml`, queries npm (`registry.npmjs.org`) and
// PyPI (`pypi.org`) for published versions, and reports any new major or
// prerelease without blocking (exit 0 on report or all-current; exit 75
// `EX_TEMPFAIL` when a registry is unreachable).
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const INCONCLUSIVE_EXIT = 75;

export type Ecosystem = "npm" | "pypi";

export interface ParsedVersion {
  readonly raw: string;
  readonly major: number;
  readonly minor: number;
  readonly patch: number;
  readonly prerelease: string | null;
}

export interface DeclaredRangeBounds {
  readonly minMajor: number;
  readonly maxMajor: number;
  readonly lowestVersion: string;
  readonly allowedMajors: readonly number[];
}

export interface PeerVersionPlan {
  readonly lowest: string;
  readonly highest: string;
  readonly interiorMajors: readonly string[];
  readonly selected: readonly string[];
}

export interface RegistryClassification {
  readonly declaredRange: string;
  readonly latestInRange: string | null;
  readonly newMajors: readonly string[];
  readonly latestNewMajor: string | null;
  readonly prereleases: readonly string[];
  readonly latestPrerelease: string | null;
}

export interface AdapterPeerEntry {
  readonly product: string;
  readonly adapter: string;
  readonly adapterDir: string;
  readonly packageName: string;
  readonly ecosystem: Ecosystem;
  readonly peerName: string;
  readonly range: string;
}

export interface AdapterPeerCheckResult {
  readonly entry: AdapterPeerEntry;
  readonly classification: RegistryClassification;
}

export interface AdaptersLatestOutcome {
  readonly exitCode: number;
  readonly results: readonly AdapterPeerCheckResult[];
  readonly report: string;
}

const SEMVER_RE = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/;
const PEP440_RE = /^(\d+)\.(\d+)(?:\.(\d+))?(?:(a|b|rc|alpha|beta|dev|post)\.?\d+)?$/i;

export function parseVersion(raw: string, ecosystem: Ecosystem): ParsedVersion | null {
  const trimmed = raw.trim();
  if (ecosystem === "npm") {
    const match = SEMVER_RE.exec(trimmed);
    if (match === null) {
      return null;
    }
    return {
      raw: trimmed,
      major: Number(match[1]),
      minor: Number(match[2]),
      patch: Number(match[3]),
      prerelease: match[4] ?? null,
    };
  }
  const match = PEP440_RE.exec(trimmed);
  if (match === null) {
    return null;
  }
  const tag = match[4]?.toLowerCase() ?? null;
  const isPrerelease = tag !== null && tag !== "post";
  return {
    raw: trimmed,
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: match[3] === undefined ? 0 : Number(match[3]),
    prerelease: isPrerelease ? tag : null,
  };
}

export function compareVersions(a: ParsedVersion, b: ParsedVersion): number {
  if (a.major !== b.major) return a.major - b.major;
  if (a.minor !== b.minor) return a.minor - b.minor;
  if (a.patch !== b.patch) return a.patch - b.patch;
  if (a.prerelease === b.prerelease) return a.raw.localeCompare(b.raw);
  if (a.prerelease === null) return 1;
  if (b.prerelease === null) return -1;
  return a.prerelease.localeCompare(b.prerelease);
}

export function parseDeclaredRange(range: string, ecosystem: Ecosystem): DeclaredRangeBounds {
  if (ecosystem === "npm") {
    const branches = range
      .split("||")
      .map((part) => part.trim())
      .filter((part) => part.length > 0);
    const majors: number[] = [];
    let lowest: ParsedVersion | null = null;
    for (const branch of branches) {
      const caret = /^\^(\d+)\.(\d+)\.(\d+)$/.exec(branch);
      if (caret === null) {
        throw new Error(`Unsupported npm peer range branch: ${branch} (in ${range})`);
      }
      const parsed: ParsedVersion = {
        raw: `${caret[1]}.${caret[2]}.${caret[3]}`,
        major: Number(caret[1]),
        minor: Number(caret[2]),
        patch: Number(caret[3]),
        prerelease: null,
      };
      majors.push(parsed.major);
      if (lowest === null || compareVersions(parsed, lowest) < 0) {
        lowest = parsed;
      }
    }
    majors.sort((a, b) => a - b);
    const minMajor = majors[0];
    const maxMajor = majors[majors.length - 1];
    if (minMajor === undefined || maxMajor === undefined || lowest === null) {
      throw new Error(`Empty npm peer range: ${range}`);
    }
    return {
      minMajor,
      maxMajor,
      lowestVersion: lowest.raw,
      allowedMajors: [...new Set(majors)],
    };
  }

  const lowerMatch = />=\s*(\d+)\.(\d+)(?:\.(\d+))?/.exec(range);
  const upperMatch = /<\s*(\d+)(?:\.(\d+))?(?:\.(\d+))?/.exec(range);
  if (lowerMatch === null || upperMatch === null) {
    throw new Error(`Unsupported Python peer range: ${range}`);
  }
  const minMajor = Number(lowerMatch[1]);
  const maxMajorExclusive = Number(upperMatch[1]);
  const maxMajor = maxMajorExclusive - 1;
  if (maxMajor < minMajor) {
    throw new Error(`Invalid Python peer range bounds: ${range}`);
  }
  const allowedMajors: number[] = [];
  for (let m = minMajor; m <= maxMajor; m += 1) {
    allowedMajors.push(m);
  }
  const patch = lowerMatch[3] === undefined ? "0" : lowerMatch[3];
  return {
    minMajor,
    maxMajor,
    lowestVersion: `${lowerMatch[1]}.${lowerMatch[2]}.${patch}`,
    allowedMajors,
  };
}

export function planPeerVersions(
  range: string,
  ecosystem: Ecosystem,
  publishedVersions: readonly string[],
): PeerVersionPlan {
  const bounds = parseDeclaredRange(range, ecosystem);
  const lowestParsed = parseVersion(bounds.lowestVersion, ecosystem);
  const stableInRange = publishedVersions
    .map((v) => parseVersion(v, ecosystem))
    .filter(
      (v): v is ParsedVersion =>
        v !== null &&
        v.prerelease === null &&
        bounds.allowedMajors.includes(v.major) &&
        (lowestParsed === null || compareVersions(v, lowestParsed) >= 0),
    )
    .toSorted(compareVersions);

  const lowest = stableInRange[0];
  const highest = stableInRange[stableInRange.length - 1];
  if (lowest === undefined || highest === undefined) {
    return {
      lowest: bounds.lowestVersion,
      highest: bounds.lowestVersion,
      interiorMajors: [],
      selected: [bounds.lowestVersion],
    };
  }

  const interiorMajors: string[] = [];
  for (const major of bounds.allowedMajors) {
    if (major === lowest.major || major === highest.major) {
      continue;
    }
    const candidates = stableInRange.filter((v) => v.major === major);
    const picked = candidates[candidates.length - 1];
    if (picked !== undefined) {
      interiorMajors.push(picked.raw);
    }
  }

  const selected = [...new Set([lowest.raw, ...interiorMajors, highest.raw])];
  return {
    lowest: lowest.raw,
    highest: highest.raw,
    interiorMajors,
    selected,
  };
}

export function classifyRegistryVersions(
  range: string,
  ecosystem: Ecosystem,
  publishedVersions: readonly string[],
): RegistryClassification {
  const bounds = parseDeclaredRange(range, ecosystem);
  const parsed = publishedVersions
    .map((v) => parseVersion(v, ecosystem))
    .filter((v): v is ParsedVersion => v !== null)
    .toSorted(compareVersions);

  const inRangeStable = parsed.filter(
    (v) => v.prerelease === null && bounds.allowedMajors.includes(v.major),
  );
  const newMajors = parsed.filter((v) => v.prerelease === null && v.major > bounds.maxMajor);
  const prereleases = parsed.filter((v) => v.prerelease !== null && v.major >= bounds.maxMajor);

  const latestInRange = inRangeStable[inRangeStable.length - 1]?.raw ?? null;
  const latestNewMajor = newMajors[newMajors.length - 1]?.raw ?? null;
  const latestPrerelease = prereleases[prereleases.length - 1]?.raw ?? null;

  return {
    declaredRange: range,
    latestInRange,
    newMajors: newMajors.map((v) => v.raw),
    latestNewMajor,
    prereleases: prereleases.map((v) => v.raw),
    latestPrerelease,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function discoverAdapterPeers(repoRoot: string): readonly AdapterPeerEntry[] {
  const adaptersRoot = join(repoRoot, "adapters");
  if (!existsSync(adaptersRoot)) {
    return [];
  }
  const entries: AdapterPeerEntry[] = [];
  for (const product of readdirSync(adaptersRoot).toSorted()) {
    const productDir = join(adaptersRoot, product);
    let children: string[];
    try {
      children = readdirSync(productDir).toSorted();
    } catch {
      continue;
    }
    for (const adapter of children) {
      const adapterDir = join(productDir, adapter);
      const pkgJsonPath = join(adapterDir, "package.json");
      if (existsSync(pkgJsonPath)) {
        const pkg: unknown = JSON.parse(readFileSync(pkgJsonPath, "utf8"));
        if (isRecord(pkg) && typeof pkg["name"] === "string" && isRecord(pkg["peerDependencies"])) {
          for (const [peerName, range] of Object.entries(pkg["peerDependencies"])) {
            if (typeof range === "string") {
              entries.push({
                product,
                adapter,
                adapterDir,
                packageName: pkg["name"],
                ecosystem: "npm",
                peerName,
                range,
              });
            }
          }
        }
      }
      const pyprojectPath = join(adapterDir, "pyproject.toml");
      if (existsSync(pyprojectPath)) {
        const toml = readFileSync(pyprojectPath, "utf8");
        const nameMatch = /^name\s*=\s*"([^"]+)"/m.exec(toml);
        const depsMatch = /dependencies\s*=\s*\[([\s\S]*?)\]/m.exec(toml);
        const packageName = nameMatch?.[1];
        const depsBlock = depsMatch?.[1];
        if (packageName !== undefined && depsBlock !== undefined) {
          const depStrings = [...depsBlock.matchAll(/"([^"]+)"/g)]
            .map((m) => m[1])
            .filter((dep): dep is string => dep !== undefined);
          for (const dep of depStrings) {
            if (dep.startsWith("goodfetch-")) {
              continue;
            }
            const split = /^([A-Za-z0-9._-]+)\s*(.+)$/.exec(dep.trim());
            const peerName = split?.[1];
            const range = split?.[2]?.trim();
            if (peerName !== undefined && range !== undefined) {
              entries.push({
                product,
                adapter,
                adapterDir,
                packageName,
                ecosystem: "pypi",
                peerName,
                range,
              });
            }
          }
        }
      }
    }
  }
  return entries;
}

async function fetchPublishedVersions(
  entry: AdapterPeerEntry,
  fetchImpl: typeof fetch,
): Promise<readonly string[]> {
  if (entry.ecosystem === "npm") {
    const url = `https://registry.npmjs.org/${encodeURIComponent(entry.peerName)}`;
    const response = await fetchImpl(url, {
      headers: { Accept: "application/vnd.npm.install-v1+json, application/json" },
    });
    if (!response.ok) {
      throw new Error(`npm registry returned ${response.status} for ${entry.peerName}`);
    }
    const body: unknown = await response.json();
    return isRecord(body) && isRecord(body["versions"]) ? Object.keys(body["versions"]) : [];
  }
  const url = `https://pypi.org/pypi/${encodeURIComponent(entry.peerName)}/json`;
  const response = await fetchImpl(url, {
    headers: { Accept: "application/json" },
  });
  if (!response.ok) {
    throw new Error(`PyPI returned ${response.status} for ${entry.peerName}`);
  }
  const body: unknown = await response.json();
  return isRecord(body) && isRecord(body["releases"]) ? Object.keys(body["releases"]) : [];
}

export function formatReport(results: readonly AdapterPeerCheckResult[]): string {
  const lines: string[] = [];
  for (const item of results) {
    const notices: string[] = [];
    if (item.classification.latestNewMajor !== null) {
      notices.push(`new major: ${item.classification.latestNewMajor}`);
    }
    if (item.classification.latestPrerelease !== null) {
      notices.push(`prerelease: ${item.classification.latestPrerelease}`);
    }
    const summary = notices.length === 0 ? "up to date" : notices.join(", ");
    lines.push(
      `${item.entry.packageName} (${item.entry.ecosystem}) -> ${item.entry.peerName} ${item.entry.range}: ${summary}`,
    );
  }
  return lines.join("\n");
}

export async function runAdaptersLatest(options?: {
  readonly repoRoot?: string;
  readonly fetchImpl?: typeof fetch;
}): Promise<AdaptersLatestOutcome> {
  const root = options?.repoRoot ?? resolve(fileURLToPath(new URL("../..", import.meta.url)));
  const fetchImpl = options?.fetchImpl ?? globalThis.fetch;
  const peers = discoverAdapterPeers(root);
  const results: AdapterPeerCheckResult[] = [];
  for (const entry of peers) {
    let versions: readonly string[];
    try {
      versions = await fetchPublishedVersions(entry, fetchImpl);
    } catch (error) {
      return {
        exitCode: INCONCLUSIVE_EXIT,
        results,
        report: `inconclusive: ${error instanceof Error ? error.message : String(error)}`,
      };
    }
    const classification = classifyRegistryVersions(entry.range, entry.ecosystem, versions);
    results.push({ entry, classification });
  }
  return {
    exitCode: 0,
    results,
    report: formatReport(results),
  };
}

async function main(): Promise<void> {
  const outcome = await runAdaptersLatest();
  if (outcome.exitCode === INCONCLUSIVE_EXIT) {
    process.stderr.write(`${outcome.report}\n`);
  } else {
    process.stdout.write(`${outcome.report}\n`);
  }
  process.exit(outcome.exitCode);
}

if (
  process.argv[1] !== undefined &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  void main();
}
