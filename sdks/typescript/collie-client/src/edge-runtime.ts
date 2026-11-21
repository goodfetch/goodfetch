// Edge runtimes are Clients only (ADR 0049): Cloudflare Workers, Vercel's
// edge runtime and Deno Deploy end what a program runs with its request, so
// code the engine would call after it (a crawl's handlers, an SDK handler, a
// Hook in code, a lent page) has nowhere to run. A member that takes such
// code refuses it here, before anything is sent, with
// `edge_handlers_unsupported`; that code runs as a Recipe on the Daemon
// instead. The markers are read from the program's globals at each call, with
// no Node built-in, since an edge runtime has none.
import { runtimeMessage, spelledOperation, type OperationNames } from "./goodfetch-error.ts";
import { EdgeHandlersUnsupportedError } from "./generated/errors.ts";

/** An edge runtime, by the name `edge_handlers_unsupported`'s details give it. */
export type EdgeRuntime = "cloudflare_workers" | "vercel_edge" | "deno_deploy";

/** Each edge runtime's name in a message. */
const RUNTIME_NAMES: Readonly<Record<EdgeRuntime, string>> = {
  cloudflare_workers: "Cloudflare Workers",
  vercel_edge: "Vercel's edge runtime",
  deno_deploy: "Deno Deploy",
};

const FIX =
  "Run that code as a Recipe on the Daemon, and start or poll its Run from the edge function.";

/** The globals that tell an edge runtime, as each runtime defines them. */
export interface EdgeGlobals {
  readonly navigator?: { readonly userAgent?: unknown } | undefined;
  readonly EdgeRuntime?: unknown;
  readonly Deno?: { readonly env?: { readonly get?: (name: string) => unknown } } | undefined;
}

/**
 * The edge runtime `globals` belong to, or `undefined` off one: workerd's
 * user agent, Vercel's `EdgeRuntime` global, or Deno Deploy's deployment ID.
 */
export function edgeRuntimeOf(globals: EdgeGlobals): EdgeRuntime | undefined {
  if (globals.navigator?.userAgent === "Cloudflare-Workers") {
    return "cloudflare_workers";
  }
  if (typeof globals.EdgeRuntime === "string") {
    return "vercel_edge";
  }
  return denoDeploymentId(globals) === undefined ? undefined : "deno_deploy";
}

/** Deno Deploy's deployment ID, or `undefined` outside Deploy or where Deno refuses to read the environment. */
function denoDeploymentId(globals: EdgeGlobals): string | undefined {
  try {
    const id = globals.Deno?.env?.get?.("DENO_DEPLOYMENT_ID");
    return typeof id === "string" && id !== "" ? id : undefined;
  } catch {
    return undefined;
  }
}

/** A member's call that hands the engine code to call after the request. */
export interface HandlersCall {
  /** The member's operation, by its catalog name, such as `crawl.start`. */
  readonly operation: string;
  /** The Product's operation names in TypeScript form, which head the message. */
  readonly names: OperationNames;
  /** The code the engine would call after the request, such as "the crawl's handlers". */
  readonly handlers: string;
}

/**
 * Fails with `edge_handlers_unsupported` when `globals`, by default the
 * program's, belong to an edge runtime, before the member sends anything;
 * passes anywhere else.
 */
export function refuseHandlersOnEdge(
  { operation, names, handlers }: HandlersCall,
  globals: EdgeGlobals = globalThis,
): void {
  const runtime = edgeRuntimeOf(globals);
  if (runtime === undefined) {
    return;
  }
  throw new EdgeHandlersUnsupportedError(
    runtimeMessage(
      `${handlers} would run after the request, which ${RUNTIME_NAMES[runtime]} ends with what it runs`,
      { operation: spelledOperation(names, operation), fix: FIX },
    ),
    { operation, fix: FIX, details: { runtime, operation } },
  );
}
