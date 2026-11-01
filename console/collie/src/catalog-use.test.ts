// The Console as an ordinary Client (spec #592; ADRs 0031 and 0068): its
// screens reach the engine only through catalog operations, named as
// literals the generated operation map lists, and never name an endpoint
// of their own; where the engine is, and how the page is signed in to it,
// is configuration, which only the build's entry, the Client's transports,
// sign-in and the offline cache's key take. So the same build runs behind
// Goodfetch Cloud's gateway. The check reads the Console's own source, as
// it ships, and is proved on a screen that breaks each rule.
import { readdirSync, readFileSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { describe, expect, test } from "vitest";
import { FOXHOUND_OPERATIONS } from "#generated/foxhound.ts";
import { operationClasses } from "#generated/operations.ts";

/** The modules that take the engine's endpoint and authentication as configuration. */
const CONFIGURATION = ["main.tsx", "sign-in.ts", "offline/offline.ts"];

/** The folders that are no screen: the Client itself and generated code. */
const NOT_SCREENS = ["client/", "generated/"];

/** What a screen does that an ordinary Client may not. */
interface Finding {
  readonly file: string;
  readonly rule: "operation_outside_catalog" | "operation_not_literal" | "endpoint_hard_coded";
  readonly found: string;
}

/** A source file, by its path below `src/`, with its text. */
interface Source {
  readonly file: string;
  readonly text: string;
}

const CATALOG: ReadonlySet<string> = new Set(Object.keys(operationClasses));
const FOXHOUND: ReadonlySet<string> = new Set(FOXHOUND_OPERATIONS);

/** A call of the Client: its method, then its first argument. */
const CALL = /\.(call|callFoxhound|subscribe)\(\s*("[^"]*"|'[^']*'|`[^`]*`|[^,)\s]+)/g;

/** An absolute address in a string. */
const ABSOLUTE = /["'`]((?:https?|wss?):\/\/[^"'`]*)/g;

/** A request of the page's own making. */
const NETWORK = /\b(fetch\(|new WebSocket\b|new EventSource\b|XMLHttpRequest\b|sendBeacon\b)/g;

/** A path the engine serves beside the Console's routes. */
const ENGINE_PATH =
  /["'`](\/(?:protocol|sign-in|session|sign-out|offline-key|content-address|csp-report)\b[^"'`]*)/g;

/** What `sources` do that an ordinary Client's screens may not. */
function findings(sources: readonly Source[]): Finding[] {
  const found: Finding[] = [];
  for (const { file, text } of sources) {
    if (NOT_SCREENS.some((folder) => file.startsWith(folder))) {
      continue;
    }
    for (const [, method, argument = ""] of text.matchAll(CALL)) {
      const literal = /^["'`]/.test(argument) && !argument.includes("${");
      const name = literal ? argument.slice(1, -1) : argument;
      if (method === "callFoxhound") {
        // The generated list types an operation held in a variable.
        if (literal && !FOXHOUND.has(name)) {
          found.push({ file, rule: "operation_outside_catalog", found: name });
        }
      } else if (!literal) {
        found.push({ file, rule: "operation_not_literal", found: argument });
      } else if (!CATALOG.has(method === "subscribe" ? `${name}.subscribe` : name)) {
        found.push({ file, rule: "operation_outside_catalog", found: name });
      }
    }
    for (const [, address = ""] of text.matchAll(ABSOLUTE)) {
      found.push({ file, rule: "endpoint_hard_coded", found: address });
    }
    if (!CONFIGURATION.includes(file)) {
      for (const [, request = ""] of text.matchAll(NETWORK)) {
        found.push({ file, rule: "endpoint_hard_coded", found: request });
      }
      for (const [, path = ""] of text.matchAll(ENGINE_PATH)) {
        found.push({ file, rule: "endpoint_hard_coded", found: path });
      }
    }
  }
  return found;
}

/** Every source file the Console's build is made from, tests and stories aside. */
function consoleSources(): Source[] {
  const root = import.meta.dirname;
  return readdirSync(root, { recursive: true, encoding: "utf8" })
    .filter((path) => /\.tsx?$/.test(path) && !/\.(test|stories)\.tsx?$/.test(path))
    .map((path) => ({
      file: relative(root, join(root, path)).split(sep).join("/"),
      text: readFileSync(join(root, path), "utf8"),
    }));
}

describe("the Console's screens", () => {
  test(
    "call only catalog operations and name no endpoint of their own",
    { tags: ["story:12-137"] },
    () => {
      const sources = consoleSources();

      expect(sources.map(({ file }) => file)).toContain("runs/runs.tsx");
      expect(findings(sources)).toEqual([]);
    },
  );

  test(
    "a screen calling an operation outside the catalog, or an endpoint of its own, is found",
    { tags: ["story:12-137"] },
    () => {
      const screen: Source = {
        file: "runs/hidden.tsx",
        text: [
          'await client.call("runs.secret_list", {});',
          "await client.call(`run.${kind}`, {});",
          'client.subscribe("gossip");',
          'await client.callFoxhound("crawl.teleport", {});',
          'await client.call("run.list", {});',
          'const socket = new WebSocket("wss://engine.example/protocol");',
          'await fetch(new URL("/session", origin));',
        ].join("\n"),
      };

      expect(findings([screen])).toEqual([
        { file: "runs/hidden.tsx", rule: "operation_outside_catalog", found: "runs.secret_list" },
        { file: "runs/hidden.tsx", rule: "operation_not_literal", found: "`run.${kind}`" },
        { file: "runs/hidden.tsx", rule: "operation_outside_catalog", found: "gossip" },
        { file: "runs/hidden.tsx", rule: "operation_outside_catalog", found: "crawl.teleport" },
        {
          file: "runs/hidden.tsx",
          rule: "endpoint_hard_coded",
          found: "wss://engine.example/protocol",
        },
        { file: "runs/hidden.tsx", rule: "endpoint_hard_coded", found: "new WebSocket" },
        { file: "runs/hidden.tsx", rule: "endpoint_hard_coded", found: "fetch(" },
        { file: "runs/hidden.tsx", rule: "endpoint_hard_coded", found: "/session" },
      ]);
    },
  );
});
