#!/usr/bin/env node
// A fixture Product's executable, standing for `goodfetch-collie` or
// `goodfetch-foxhound` in the install harness's tests: it loads its
// platform package's engine and prints `<executable> <version>`, or fails as
// the Product's bin does when the platform package is missing (ADR 0069):
// `engine_not_found`, as one `{ "error": … }` on stdout under `--json`, else
// on stderr, with the unavailable group's exit code, 8.
import { engineNotFound } from "@goodfetch/collie-client";
import manifest from "./package.json" with { type: "json" };

const executable = Object.keys(manifest.bin)[0];
const platformPackage = `${manifest.name}-${process.platform}-${process.arch}`;
const json = process.argv.slice(2).includes("--json");

let engine;
try {
  engine = await import(`${platformPackage}/engine.mjs`);
} catch (cause) {
  const error = engineNotFound(platformPackage, cause);
  const text = json
    ? JSON.stringify({ error })
    : [
        error.message,
        `code: ${error.code}`,
        `retryable: ${String(error.retryable)}`,
        `fix: ${error.fix}`,
      ].join("\n");
  const stream = json ? process.stdout : process.stderr;
  stream.write(`${text}\n`, () => process.exit(8));
}
if (engine !== undefined) {
  process.stdout.write(`${executable} ${engine.version}\n`);
}
