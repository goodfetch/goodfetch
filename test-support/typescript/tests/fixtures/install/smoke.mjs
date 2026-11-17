// The fixture Products' smoke script: each package named after the
// expected version imports, and its version is that one. It uses only what
// Node, Bun and Deno all carry.
import { strict as assert } from "node:assert";

const [expected, ...packages] = process.argv.slice(2);
for (const name of packages) {
  const { version } = await import(name);
  assert.equal(version, expected, `${name}'s installed version`);
}
process.stdout.write(`${packages.join(", ")} ${expected}\n`);
