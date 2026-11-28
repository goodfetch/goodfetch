// Post-publish Live check `n8n-scanner` (ADRs 0064 and 0070; spec #608,
// issue 1405).
//
// Verifies `@goodfetch/n8n-nodes-collie` against n8n's verified community node
// scanner rules after each nightly or release publish:
// - Queries npm for `@goodfetch/n8n-nodes-collie` (or checks the local package
//   directly when `GOODFETCH_N8N_SCANNER_LOCAL=1`).
// - Before the first publish (404 on npm), exits 0 with a clear pre-publish
//   status line so nightly Live checks stay clean.
// - When npm is unreachable or returns 5xx, exits 75 (`EX_TEMPFAIL`,
//   inconclusive per ADR 0064).
// - When published, checks verified-node metadata (`keywords`, `n8n.strict`,
//   zero runtime `dependencies`) and exits 1 on any violation.
import { execFileSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const INCONCLUSIVE_EXIT = 75;
export const N8N_PACKAGE_NAME = "@goodfetch/n8n-nodes-collie";

export interface N8nScannerOutcome {
  readonly exitCode: number;
  readonly report: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function validatePublishedManifest(manifest: Record<string, unknown>): string[] {
  const issues: string[] = [];
  const name = manifest["name"];
  if (name !== N8N_PACKAGE_NAME) {
    issues.push(`expected package name ${N8N_PACKAGE_NAME}, got ${String(name)}`);
  }
  const deps = manifest["dependencies"];
  if (isRecord(deps) && Object.keys(deps).length > 0) {
    issues.push(`published node has runtime dependencies: ${Object.keys(deps).join(", ")}`);
  }
  const keywords = manifest["keywords"];
  if (!Array.isArray(keywords) || !keywords.includes("n8n-community-node-package")) {
    issues.push('published node is missing keyword "n8n-community-node-package"');
  }
  const n8n = manifest["n8n"];
  if (!isRecord(n8n) || n8n["strict"] !== true || n8n["n8nNodesApiVersion"] !== 1) {
    issues.push("published node must declare n8n.strict=true and n8n.n8nNodesApiVersion=1");
  }
  return issues;
}

export async function runN8nScanner(
  options: {
    readonly localOnly?: boolean;
    readonly packageDir?: string;
    readonly fetchImpl?: typeof fetch;
  } = {},
): Promise<N8nScannerOutcome> {
  const repoRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));
  const packageDir = options.packageDir ?? resolve(repoRoot, "integrations/collie/n8n");

  try {
    // `eslint-plugin-n8n-nodes-base` switches into its own internal test-fixture
    // mode when `NODE_ENV=test` (set by Vitest). Strip `NODE_ENV` so real rule
    // checks run identically from tests and from the shell.
    const lintEnv: NodeJS.ProcessEnv = { ...process.env };
    delete lintEnv["NODE_ENV"];
    execFileSync("pnpm", ["--dir", packageDir, "run", "lint"], {
      stdio: "pipe",
      encoding: "utf8",
      timeout: 60_000,
      env: lintEnv,
    });
  } catch (error) {
    const details =
      error !== null && typeof error === "object" && "stdout" in error && "stderr" in error
        ? `${String(error.stdout)}\n${String(error.stderr)}`.trim()
        : error instanceof Error
          ? error.message
          : String(error);
    return {
      exitCode: 1,
      report: `n8n-scanner: local verified-node lint failed:\n${details}`,
    };
  }

  if (options.localOnly === true) {
    return {
      exitCode: 0,
      report: `n8n-scanner: PASS (local verified-node check for ${N8N_PACKAGE_NAME})`,
    };
  }

  const fetchImpl = options.fetchImpl ?? globalThis.fetch;
  const registryUrl = `https://registry.npmjs.org/${encodeURIComponent(N8N_PACKAGE_NAME)}`;
  let response: Response;
  try {
    response = await fetchImpl(registryUrl, {
      headers: { Accept: "application/json" },
      signal: AbortSignal.timeout(15_000),
    });
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    return {
      exitCode: INCONCLUSIVE_EXIT,
      report: `n8n-scanner: inconclusive (npm registry unreachable: ${detail})`,
    };
  }

  if (response.status === 404) {
    return {
      exitCode: 0,
      report: `n8n-scanner: ${N8N_PACKAGE_NAME} not yet published on npm; local verified-node check passed`,
    };
  }
  if (!response.ok) {
    return {
      exitCode: INCONCLUSIVE_EXIT,
      report: `n8n-scanner: inconclusive (npm registry returned HTTP ${response.status})`,
    };
  }

  let metadata: unknown;
  try {
    metadata = await response.json();
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    return {
      exitCode: INCONCLUSIVE_EXIT,
      report: `n8n-scanner: inconclusive (invalid registry JSON: ${detail})`,
    };
  }

  if (!isRecord(metadata)) {
    return {
      exitCode: 1,
      report: "n8n-scanner: registry returned a non-object metadata payload",
    };
  }

  const distTags = isRecord(metadata["dist-tags"]) ? metadata["dist-tags"] : undefined;
  const latestVersion = typeof distTags?.["latest"] === "string" ? distTags["latest"] : undefined;
  const versions = isRecord(metadata["versions"]) ? metadata["versions"] : undefined;
  const latestManifest =
    latestVersion !== undefined && versions !== undefined && isRecord(versions[latestVersion])
      ? versions[latestVersion]
      : undefined;

  if (latestManifest === undefined) {
    return {
      exitCode: 1,
      report: `n8n-scanner: missing latest version manifest for ${N8N_PACKAGE_NAME}`,
    };
  }

  const issues = validatePublishedManifest(latestManifest);
  if (issues.length > 0) {
    return {
      exitCode: 1,
      report: `n8n-scanner: FAIL for ${N8N_PACKAGE_NAME}@${latestVersion}:\n${issues.join("\n")}`,
    };
  }

  return {
    exitCode: 0,
    report: `n8n-scanner: PASS (${N8N_PACKAGE_NAME}@${latestVersion} verified-node scan clean)`,
  };
}

async function main(): Promise<void> {
  const outcome = await runN8nScanner({
    localOnly: process.env["GOODFETCH_N8N_SCANNER_LOCAL"] === "1",
  });
  process.stdout.write(`${outcome.report}\n`);
  process.exitCode = outcome.exitCode;
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
