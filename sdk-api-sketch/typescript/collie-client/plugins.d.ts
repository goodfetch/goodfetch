// SDK handlers both Products take (ADR 0104): a program's own Proxy provider, registered on
// `connect()` by name as ADR 0101 registers `modelProviders` (models.d.ts) and ADR 0092 takes
// `resolveSecret` (sessions.d.ts). The Fetch backend and Sink kinds are Foxhound's (ADR 0088), in
// `@goodfetch/foxhound`'s hooks.d.ts, which states the rules every SDK handler follows.

import type { BlockCause, HandlerFailedDetails, ProgramError } from "./results.js";
import type { SecretReference } from "./sessions.js";

/** What a Proxy provider is asked for: one proxy per Session (ADRs 0021, 0092). */
export interface ProxyRequest {
  readonly sessionId: string;
  /** The registrable domain of the Session's first request. */
  readonly domain: string;
  /** Set when the Session replaces one retired for this Block cause. */
  readonly replacing: BlockCause | null;
}
/** A proxy, with Playwright's fields as `proxy.*` has them (ADR 0092). */
export interface ProxyAnswer {
  readonly server: string;
  readonly username?: string;
  /** A Secret reference, never a value: `caller:` resolves through `resolveSecret`. */
  readonly password?: SecretReference;
}
export type ProxyProviderHandler = (request: ProxyRequest) => Promise<ProxyAnswer>;

/** Details of a `<kind>_handler_failed`: the handler, then the exception as `handler_failed` carries a handler's. */
export interface SdkHandlerFailedDetails extends HandlerFailedDetails {
  /** The name the handler was registered under. */
  readonly handler: string;
}
/** A Proxy provider handler threw something other than a GoodfetchError: the Session does not open. In a crawl, the requests that needed it are retried only when `crawl.retry.handler_failed` is on for their Label; in a Collie Run, the call fails. */
export declare class ProxyHandlerFailedError extends ProgramError {
  readonly code: "proxy_handler_failed";
  readonly details: SdkHandlerFailedDetails;
}
