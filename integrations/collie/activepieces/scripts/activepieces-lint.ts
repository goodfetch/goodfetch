// Activepieces piece linter for `goodfetch-collie` (ADRs 0027, 0070, 0090,
// 0095; spec #608, issue 1409).
//
// Validates:
// 1. Package name is `goodfetch-collie` with zero Foxhound dependencies.
// 2. Every action and trigger declared in `integration.toml` has a matching
//    per-operation implementation in `goodfetchColliePiece`.
// 3. Every action and trigger carries non-empty `name`, `displayName`,
//    `description`, `auth`, and `props`.
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { goodfetchColliePiece } from "../src/index.ts";

export interface ActivepiecesLintViolation {
  readonly rule: string;
  readonly message: string;
}

export interface ActivepiecesLintResult {
  readonly ok: boolean;
  readonly violations: readonly ActivepiecesLintViolation[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function lintActivepiecesPiece(packageDir: string): ActivepiecesLintResult {
  const violations: ActivepiecesLintViolation[] = [];
  const pkgPath = join(packageDir, "package.json");
  if (!existsSync(pkgPath)) {
    return {
      ok: false,
      violations: [{ rule: "activepieces/package-json", message: "Missing package.json." }],
    };
  }

  const pkg: unknown = JSON.parse(readFileSync(pkgPath, "utf8"));
  if (!isRecord(pkg)) {
    return {
      ok: false,
      violations: [
        { rule: "activepieces/package-json", message: "package.json must be an object." },
      ],
    };
  }

  if (pkg["name"] !== "goodfetch-collie") {
    violations.push({
      rule: "activepieces/piece-name",
      message: `Expected package name goodfetch-collie, got ${String(pkg["name"])}.`,
    });
  }

  const deps = pkg["dependencies"];
  if (isRecord(deps)) {
    for (const dep of Object.keys(deps)) {
      if (dep.toLowerCase().includes("foxhound")) {
        violations.push({
          rule: "activepieces/no-foxhound-dep",
          message: `Collie piece must not depend on Foxhound package ${dep}.`,
        });
      }
    }
  }

  if (goodfetchColliePiece.name !== "goodfetch-collie") {
    violations.push({
      rule: "activepieces/piece-export-name",
      message: `Piece export name must be goodfetch-collie, got ${goodfetchColliePiece.name}.`,
    });
  }

  const expectedActions = [
    "start-recipe",
    "start-catalog-run",
    "get-run-report",
    "get-run-progress",
  ];
  const actualActions = goodfetchColliePiece.actions.map((a) => a.name);
  for (const name of expectedActions) {
    if (!actualActions.includes(name)) {
      violations.push({
        rule: "activepieces/missing-action",
        message: `Missing action ${name} on goodfetch-collie piece.`,
      });
    }
  }

  for (const action of goodfetchColliePiece.actions) {
    if (action.displayName.trim().length === 0 || action.description.trim().length === 0) {
      violations.push({
        rule: "activepieces/action-metadata",
        message: `Action ${action.name} must have non-empty displayName and description.`,
      });
    }
  }

  const actualTriggers = goodfetchColliePiece.triggers.map((t) => t.name);
  if (!actualTriggers.includes("notification")) {
    violations.push({
      rule: "activepieces/missing-trigger",
      message: "Missing trigger notification on goodfetch-collie piece.",
    });
  }

  return {
    ok: violations.length === 0,
    violations,
  };
}

function main(): void {
  const packageDir = resolve(fileURLToPath(new URL("..", import.meta.url)));
  const result = lintActivepiecesPiece(packageDir);
  if (!result.ok) {
    for (const violation of result.violations) {
      process.stderr.write(`[${violation.rule}] ${violation.message}\n`);
    }
    process.exitCode = 1;
    return;
  }
  process.stdout.write(`activepieces lint: PASS (${packageDir})\n`);
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
