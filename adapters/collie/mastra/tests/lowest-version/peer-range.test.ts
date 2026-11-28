// Lowest- and highest-version peer range check for `@goodfetch/collie-mastra`
// (ADR 0070; spec #608, issue 1382).
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { collieTools } from "@goodfetch/collie-mastra";
import type { Connection, JSONValue, RawCallOptions, RawParams } from "@goodfetch/collie";
import { describe, expect, test } from "vitest";

const localRequire = createRequire(import.meta.url);
const adapterRequire = createRequire(new URL("../../package.json", import.meta.url));

function readPkgVersion(pkgJsonPath: string): string {
  const parsed: unknown = JSON.parse(readFileSync(pkgJsonPath, "utf8"));
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new TypeError(`expected package.json object at ${pkgJsonPath}`);
  }
  const version = Reflect.get(parsed, "version");
  if (typeof version !== "string") {
    throw new TypeError(`missing version in ${pkgJsonPath}`);
  }
  return version;
}

describe("@goodfetch/collie-mastra peer range cells", () => {
  test(
    "runs against the pinned lowest peer version (@mastra/core@1.0.0) and matches the highest lockfile peer",
    { tags: ["adapter:mastra", "story:11-244"] },
    async () => {
      const lowestVersion = readPkgVersion(localRequire.resolve("@mastra/core/package.json"));
      const highestVersion = readPkgVersion(adapterRequire.resolve("@mastra/core/package.json"));
      expect(lowestVersion).toBe("1.0.0");
      expect(highestVersion.startsWith("1.")).toBe(true);

      const calls: {
        operation: string;
        params: RawParams | undefined;
        options: RawCallOptions | undefined;
      }[] = [];
      const connection: Pick<Connection, "call"> = {
        call: async (operation, params, options): Promise<JSONValue> => {
          calls.push({ operation, params, options });
          return { run_id: "r-lowest" };
        },
      };

      const tools = collieTools(connection, { profile: "browser" });
      const runStart = tools["run_start"];
      if (runStart === undefined || runStart.execute === undefined) {
        throw new Error("expected run_start tool with execute handler");
      }
      const output = await runStart.execute({}, {});
      expect(output).toEqual({ ok: true, result: { run_id: "r-lowest" } });
      expect(calls[0]?.operation).toBe("run.start");
      expect(calls[0]?.options?.form).toBe("tool");

      const lowestManifest = readFileSync(join(import.meta.dirname, "package.json"), "utf8");
      expect(lowestManifest).toContain('"@mastra/core": "1.0.0"');
    },
  );
});
