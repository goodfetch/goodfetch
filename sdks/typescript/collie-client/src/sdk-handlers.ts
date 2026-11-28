// SDK handlers (ADRs 0033, 0104 and 0132): serves incoming Plugin-kind
// requests from the engine to the program's registered handlers, keeping a
// `GoodfetchError`'s code, wrapping any other exception in the kind's
// `<kind>_handler_failed` with its type, message and stack, cancelling on
// `$/cancel`, and making the request's `_meta` Trace context current around
// the handler's code.
import type { Served } from "./client-connection.ts";
import { isRecord } from "./framing.ts";
import {
  ExtractHandlerFailedError,
  ExtractorUnavailableError,
  GoodfetchError,
  type ExtractContentParams,
  type ExtractContentResult,
  type ExtractHandler,
} from "./generated/index.ts";
import { thrownException } from "./thrown.ts";
import { extractTraceContext, type TraceContextRunner } from "./trace-context.ts";

/** The `hello.handlers` map declaring registered SDK handler names by kind. */
export function declaredHandlers(
  extractors: Readonly<Record<string, ExtractHandler>> | undefined,
): Readonly<Record<string, string[]>> | undefined {
  if (extractors === undefined) {
    return undefined;
  }
  const names = Object.keys(extractors);
  return names.length === 0 ? undefined : { extract: names };
}

/** An Extractor handler that may also accept `{ signal }` for cancellation. */
interface CancellableExtractHandler {
  content(
    params: ExtractContentParams,
    options?: { readonly signal?: AbortSignal },
  ): Promise<ExtractContentResult>;
}

/**
 * Serves the engine's `extract.content` requests to the Connection's
 * registered Extractor handlers (ADR 0104).
 */
export function serveExtractors(
  extractors: Readonly<Record<string, ExtractHandler>>,
  runner: () => TraceContextRunner,
): Served {
  return async (rawParams: unknown, signal: AbortSignal): Promise<unknown> => {
    const meta = extractTraceContext(rawParams);
    const fields = isRecord(rawParams) ? rawParams : {};
    const handlerName = typeof fields["handler"] === "string" ? fields["handler"] : "";
    const url = typeof fields["url"] === "string" ? fields["url"] : "";
    const html = typeof fields["html"] === "string" ? fields["html"] : "";
    const handler: CancellableExtractHandler | undefined = Object.hasOwn(extractors, handlerName)
      ? extractors[handlerName]
      : undefined;
    if (handler === undefined) {
      throw new ExtractorUnavailableError(
        `no Extractor SDK handler named \`${handlerName}\` is registered on this Connection`,
        { details: { extractor: handlerName } },
      );
    }
    const callParams: ExtractContentParams = { url, html };
    const result = await runner()(meta, async () => {
      try {
        return await handler.content(callParams, { signal });
      } catch (error) {
        if (error instanceof GoodfetchError) {
          throw error;
        }
        const thrown = thrownException(error);
        const stack = error instanceof Error ? (error.stack ?? "") : "";
        throw new ExtractHandlerFailedError(
          `Extractor SDK handler \`${handlerName}\` failed with ${thrown.type}: ${thrown.message}`,
          {
            details: {
              extractor: handlerName,
              handler: handlerName,
              url,
              exception_type: thrown.type,
              exception_message: thrown.message,
              exception_stack: stack,
            },
            cause: error,
          },
        );
      }
    });
    return {
      title: result.title,
      text: result.text,
      markdown: result.markdown,
      metadata: result.metadata,
    };
  };
}
