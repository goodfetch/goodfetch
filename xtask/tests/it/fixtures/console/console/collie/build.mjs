// Writes a page, a script and a binary file into the folder `--outDir`
// names, as Vite's build does, and counts its runs in the workspace's
// `target/console-build-runs.txt`, which no build reads.
import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const flag = process.argv.indexOf("--outDir");
const out = process.argv[flag + 1];
mkdirSync(join(out, "assets"), { recursive: true });
writeFileSync(join(out, "index.html"), "<!doctype html><title>Goodfetch Collie</title>\n");
writeFileSync(join(out, "assets", "index-1a2b.js"), "console.log('Collie');\n".repeat(64));
writeFileSync(join(out, "assets", "font-3c4d.woff2"), Buffer.from([0, 1, 2, 3, 255, 254, 253]));
const target = join(process.cwd(), "..", "..", "target");
mkdirSync(target, { recursive: true });
appendFileSync(join(target, "console-build-runs.txt"), "run\n");
