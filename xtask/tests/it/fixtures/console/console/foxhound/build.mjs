// Writes Foxhound's page into the folder `--outDir` names, as Vite's build
// does, and counts its runs in the workspace's
// `target/console-foxhound-build-runs.txt`, which no build reads.
import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const flag = process.argv.indexOf("--outDir");
const out = process.argv[flag + 1];
mkdirSync(out, { recursive: true });
writeFileSync(join(out, "index.html"), "<!doctype html><title>Goodfetch Foxhound</title>\n");
const target = join(process.cwd(), "..", "..", "target");
mkdirSync(target, { recursive: true });
appendFileSync(join(target, "console-foxhound-build-runs.txt"), "run\n");
