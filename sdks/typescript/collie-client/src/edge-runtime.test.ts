// The edge runtime by each marker, given as each runtime defines its global
// (ADR 0062: passed in, never patched onto this process's globals).
import { describe, expect, test } from "vitest";
import {
  edgeRuntimeOf,
  refuseHandlersOnEdge,
  type EdgeGlobals,
  type HandlersCall,
} from "./edge-runtime.ts";
import { EdgeHandlersUnsupportedError } from "./generated/errors.ts";

/** Deno's globals, its environment holding only `variables`. */
function denoWith(variables: Readonly<Record<string, string>>): EdgeGlobals {
  return { Deno: { env: { get: (name: string) => variables[name] } } };
}

const CRAWL_START: HandlersCall = {
  operation: "crawl.start",
  names: { "crawl.start": "crawl.start" },
  handlers: "the crawl's handlers",
};

/** The error `refuse` throws, or `undefined` when it passes. */
function thrownBy(refuse: () => void): unknown {
  try {
    refuse();
  } catch (error) {
    return error;
  }
  return undefined;
}

describe("the edge runtime a program runs on", { tags: ["story:11-7"] }, () => {
  test("is Cloudflare Workers by workerd's user agent", () => {
    expect(edgeRuntimeOf({ navigator: { userAgent: "Cloudflare-Workers" } })).toBe(
      "cloudflare_workers",
    );
  });

  test("is Vercel's edge runtime by its `EdgeRuntime` global", () => {
    expect(edgeRuntimeOf({ EdgeRuntime: "edge-runtime" })).toBe("vercel_edge");
  });

  test("is Deno Deploy by its deployment ID", () => {
    expect(edgeRuntimeOf(denoWith({ DENO_DEPLOYMENT_ID: "a1b2c3" }))).toBe("deno_deploy");
  });

  test("is none on this Node process", () => {
    expect(edgeRuntimeOf(globalThis)).toBeUndefined();
  });

  test("is none on Deno outside Deploy, even where Deno refuses to read the environment", () => {
    expect(edgeRuntimeOf(denoWith({}))).toBeUndefined();
    const refusing: EdgeGlobals = {
      Deno: {
        env: {
          get: () => {
            throw new Error("Requires env access to DENO_DEPLOYMENT_ID");
          },
        },
      },
    };
    expect(edgeRuntimeOf(refusing)).toBeUndefined();
  });
});

describe(
  "code the engine would call after the request, asked for on an edge runtime",
  { tags: ["story:11-7", "error:edge_handlers_unsupported"] },
  () => {
    test("fails with `edge_handlers_unsupported`, naming a Recipe on the Daemon", () => {
      const refused = thrownBy(() =>
        refuseHandlersOnEdge(CRAWL_START, { EdgeRuntime: "edge-runtime" }),
      );

      expect(refused).toBeInstanceOf(EdgeHandlersUnsupportedError);
      expect(refused).toMatchObject({
        code: "edge_handlers_unsupported",
        operation: "crawl.start",
        message: expect.stringMatching(
          /^crawl\.start: the crawl's handlers .*Recipe on the Daemon/,
        ),
        fix: expect.stringContaining("Recipe on the Daemon"),
        details: { runtime: "vercel_edge", operation: "crawl.start" },
      });
    });

    test("passes on this Node process", () => {
      expect(thrownBy(() => refuseHandlersOnEdge(CRAWL_START))).toBeUndefined();
    });
  },
);
