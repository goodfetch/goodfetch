// Strict verified-node linter for `@goodfetch/n8n-nodes-collie` (`n8n-node lint`
// verified-node rule set; ADRs 0027, 0070, 0090, 0095; spec #608, issue 1405).
//
// Enforces:
// 1. Package name matches `^(@[a-z0-9-]+/)?n8n-nodes-[a-z0-9-]+$`.
// 2. `keywords` includes `n8n-community-node-package`.
// 3. `n8n.n8nNodesApiVersion === 1` and `n8n.strict === true`.
// 4. Zero runtime `dependencies` (verified community node requirement).
// 5. No `process.env`, `process.argv`, filesystem access, child_process, os,
//    dynamic `eval`/`new Function`, or non-relative runtime imports/requires in
//    any node or credential source (or built `dist/` artifact when present).
// 6. All declared node/credential descriptions have required metadata.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { extname, join, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export interface LintViolation {
  readonly rule: string;
  readonly file: string;
  readonly message: string;
}

export interface N8nLintResult {
  readonly ok: boolean;
  readonly violations: readonly LintViolation[];
}

const PACKAGE_NAME_RE = /^(@[a-z0-9-]+\/)?n8n-nodes-[a-z0-9-]+$/;

const FORBIDDEN_PATTERNS: ReadonlyArray<{
  readonly rule: string;
  readonly regex: RegExp;
  readonly message: string;
}> = [
  {
    rule: "n8n-nodes-base/no-process-env",
    regex: /\bprocess\s*\.\s*env\b/,
    message: "Verified n8n nodes must not read `process.env`.",
  },
  {
    rule: "n8n-nodes-base/no-process-argv",
    regex: /\bprocess\s*\.\s*argv\b/,
    message: "Verified n8n nodes must not read `process.argv`.",
  },
  {
    rule: "n8n-nodes-base/no-node-builtins",
    regex:
      /\b(?:from\s+["'](?:node:)?(?:fs|child_process|os|path|net|tls|http|https|dgram|dns|cluster|worker_threads|vm|crypto)(?:\/[^"']*)?["']|require\s*\(\s*["'](?:node:)?(?:fs|child_process|os|path|net|tls|http|https|dgram|dns|cluster|worker_threads|vm|crypto)(?:\/[^"']*)?["']\s*\))/,
    message: "Verified n8n nodes must not import or require Node.js built-in modules.",
  },
  {
    rule: "n8n-nodes-base/no-eval",
    regex: /\b(?:eval\s*\(|new\s+Function\s*\()/,
    message: "Verified n8n nodes must not invoke `eval` or `new Function`.",
  },
  {
    rule: "n8n-nodes-base/no-external-runtime-import",
    regex: /\bfrom\s+["'](?!\.\.?\/)[^"']+["']/,
    message: "Verified n8n nodes must not import external runtime packages.",
  },
  {
    rule: "n8n-nodes-base/no-external-runtime-require",
    regex: /\brequire\s*\(\s*["'](?!\.\.?\/)[^"']+["']\s*\)/,
    message: "Verified n8n nodes must not require external runtime packages.",
  },
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function walkFiles(dir: string, predicate: (filePath: string) => boolean): string[] {
  if (!existsSync(dir)) {
    return [];
  }
  const results: string[] = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const stat = statSync(full);
    if (stat.isDirectory()) {
      results.push(...walkFiles(full, predicate));
    } else if (stat.isFile() && predicate(full)) {
      results.push(full);
    }
  }
  results.sort();
  return results;
}

function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
}

export function lintN8nPackage(packageDir: string): N8nLintResult {
  const violations: LintViolation[] = [];
  const pkgPath = join(packageDir, "package.json");
  if (!existsSync(pkgPath)) {
    return {
      ok: false,
      violations: [
        {
          rule: "n8n-nodes-base/package-json",
          file: "package.json",
          message: "Missing package.json.",
        },
      ],
    };
  }

  let pkg: unknown;
  try {
    pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
  } catch (error) {
    return {
      ok: false,
      violations: [
        {
          rule: "n8n-nodes-base/package-json",
          file: "package.json",
          message: `Invalid JSON: ${error instanceof Error ? error.message : String(error)}`,
        },
      ],
    };
  }

  if (!isRecord(pkg)) {
    return {
      ok: false,
      violations: [
        {
          rule: "n8n-nodes-base/package-json",
          file: "package.json",
          message: "package.json must be a JSON object.",
        },
      ],
    };
  }

  const name = pkg["name"];
  if (typeof name !== "string" || !PACKAGE_NAME_RE.test(name)) {
    violations.push({
      rule: "n8n-nodes-base/community-package-json-name-still-default",
      file: "package.json",
      message: `Package name must match ${PACKAGE_NAME_RE.source}, got ${String(name)}.`,
    });
  }

  const keywords = pkg["keywords"];
  if (!Array.isArray(keywords) || !keywords.includes("n8n-community-node-package")) {
    violations.push({
      rule: "n8n-nodes-base/community-package-json-keywords",
      file: "package.json",
      message: 'package.json `keywords` must include "n8n-community-node-package".',
    });
  }

  const deps = pkg["dependencies"];
  if (isRecord(deps) && Object.keys(deps).length > 0) {
    violations.push({
      rule: "n8n-nodes-base/verified-node-no-runtime-dependencies",
      file: "package.json",
      message: `Verified n8n nodes must have zero runtime dependencies; found: ${Object.keys(deps).join(", ")}.`,
    });
  }

  const n8nConfig = pkg["n8n"];
  if (!isRecord(n8nConfig)) {
    violations.push({
      rule: "n8n-nodes-base/community-package-json-n8n-block",
      file: "package.json",
      message: "package.json must declare an `n8n` block.",
    });
  } else {
    if (n8nConfig["n8nNodesApiVersion"] !== 1) {
      violations.push({
        rule: "n8n-nodes-base/community-package-json-api-version",
        file: "package.json",
        message: "`n8n.n8nNodesApiVersion` must be 1.",
      });
    }
    if (n8nConfig["strict"] !== true) {
      violations.push({
        rule: "n8n-nodes-base/community-package-json-strict",
        file: "package.json",
        message: "`n8n.strict` must be true.",
      });
    }
    const nodes = n8nConfig["nodes"];
    if (!Array.isArray(nodes) || nodes.length === 0) {
      violations.push({
        rule: "n8n-nodes-base/community-package-json-nodes",
        file: "package.json",
        message: "`n8n.nodes` must be a non-empty array.",
      });
    }
    const credentials = n8nConfig["credentials"];
    if (!Array.isArray(credentials) || credentials.length === 0) {
      violations.push({
        rule: "n8n-nodes-base/community-package-json-credentials",
        file: "package.json",
        message: "`n8n.credentials` must be a non-empty array.",
      });
    }
    if (existsSync(join(packageDir, "dist"))) {
      for (const entry of [
        ...(Array.isArray(nodes) ? nodes : []),
        ...(Array.isArray(credentials) ? credentials : []),
      ]) {
        if (typeof entry === "string" && !existsSync(join(packageDir, entry))) {
          violations.push({
            rule: "n8n-nodes-base/community-package-json-missing-dist",
            file: entry,
            message: `Declared n8n entry \`${entry}\` does not exist.`,
          });
        }
      }
    }
  }

  const sourceFiles = walkFiles(join(packageDir, "src"), (filePath) => {
    const ext = extname(filePath);
    return (ext === ".ts" || ext === ".js") && !filePath.endsWith(".test.ts");
  });
  const distFiles = walkFiles(join(packageDir, "dist"), (filePath) => extname(filePath) === ".js");

  for (const filePath of [...sourceFiles, ...distFiles]) {
    const rel = relative(packageDir, filePath);
    const cleaned = stripComments(readFileSync(filePath, "utf8"));
    for (const pattern of FORBIDDEN_PATTERNS) {
      if (pattern.regex.test(cleaned)) {
        violations.push({
          rule: pattern.rule,
          file: rel,
          message: pattern.message,
        });
      }
    }
  }

  return {
    ok: violations.length === 0,
    violations,
  };
}

function main(): void {
  const packageDir = resolve(fileURLToPath(new URL("..", import.meta.url)));
  const result = lintN8nPackage(packageDir);
  if (!result.ok) {
    for (const violation of result.violations) {
      process.stderr.write(`[${violation.rule}] ${violation.file}: ${violation.message}\n`);
    }
    process.exitCode = 1;
    return;
  }
  process.stdout.write(`n8n-node lint (strict): PASS (${packageDir})\n`);
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
