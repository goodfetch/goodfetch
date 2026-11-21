// The environment map an SDK takes in a test (ADRs 0069, 0090 and 0094):
// the test's cleared environment, `GOODFETCH_HOME` among it, with the
// engine-path variable of the SDK's Product pointing at that Product's
// executable from the test tool manifest, or at the Scripted engine playing
// a script of the shared corpus.
import { copyFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { toolPath, type TestEnvDescription } from "./test-env.ts";

/** A Product; a test runs Collie's executable unless it needs Foxhound (ADR 0094). */
export type Product = "collie" | "foxhound";

const ENGINE_PATH_VARIABLES = {
  collie: "GOODFETCH_COLLIE_ENGINE_PATH",
  foxhound: "GOODFETCH_FOXHOUND_ENGINE_PATH",
} as const satisfies Record<Product, string>;

/** The environment map an SDK of `product` takes; `tool_missing` when the manifest lists no executable of it. */
export function productEnv(
  description: TestEnvDescription,
  product: Product,
): Readonly<Record<string, string>> {
  return {
    ...description.env,
    [ENGINE_PATH_VARIABLES[product]]: toolPath(description.tools, product),
  };
}

/** The shared corpus of engine scripts (ADR 0069). */
const corpus = new URL("../../fixtures/engine-scripts/", import.meta.url);

/** The file of the corpus's script `script`, such as `connect/reached-daemon.json`. */
export function corpusScript(script: string): URL {
  return new URL(script, corpus);
}

/**
 * The environment map under which an SDK of `product` starts
 * `goodfetch-scripted-engine` as its Attached engine, playing the corpus's
 * script `script`: the script is installed as `scripted-engine/script.json`
 * under `home`, which the map's `GOODFETCH_HOME` names, and the engine's
 * record is written beside it. Each Connection a test opens at once takes a
 * home of its own. `tool_missing` when the manifest lists no Scripted engine.
 */
export function scriptedEngineEnv(
  description: TestEnvDescription,
  product: Product,
  script: string,
  home: string = description.home,
): Readonly<Record<string, string>> {
  const engine = toolPath(description.tools, "scripted_engine");
  mkdirSync(join(home, "scripted-engine"), { recursive: true });
  copyFileSync(corpusScript(script), join(home, "scripted-engine", "script.json"));
  return {
    ...description.env,
    GOODFETCH_HOME: home,
    [ENGINE_PATH_VARIABLES[product]]: engine,
  };
}

/**
 * The methods of the Client's messages the Scripted engine under `home` has
 * read, in its order: it records each message as it reads it, one flushed
 * JSON line each. Empty before the engine writes its record.
 */
export function scriptedEngineClientMethods(home: string): unknown[] {
  const record = join(home, "scripted-engine", "record.jsonl");
  if (!existsSync(record)) {
    return [];
  }
  return readFileSync(record, "utf8")
    .split("\n")
    .filter((line) => line !== "")
    .map((line): unknown => JSON.parse(line))
    .filter(
      (entry): entry is { from: string; message: { method?: unknown } } =>
        typeof entry === "object" && entry !== null && "from" in entry && entry.from === "client",
    )
    .map((entry) => entry.message.method);
}
