// The response a page's request got, such as a navigation's document (ADR
// 0085): what `page.goto`, `reload`, `goBack`, `goForward` and `request`
// answer. Its head is in hand; its body is read on demand, through
// `response.body`, from the file the engine keeps it in, which a Client on
// the same machine reads (ADR 0118). It keeps Playwright's methods; Rust's
// `Response` and Python's are its twins.

import { readFile } from "node:fs/promises";
import { jsonOf, text } from "./decoding.ts";
import { BodyUnavailableError, ResponseNotJSONError } from "./generated/errors.ts";
import type { CollieOperationTypes } from "./generated/operations.ts";
import type * as types from "./generated/types.ts";
import type { HandleHost } from "./handle.ts";
import type { JSONValue } from "./json.ts";
import { warnExperimental } from "./markers.ts";

const BODY = "response.body";

/** The bytes of the body `handle` names, from the file the engine keeps it in. */
async function read(handle: types.BodyHandle): Promise<Uint8Array> {
  let body: Uint8Array;
  try {
    body = await readFile(handle.path);
  } catch (error) {
    throw unreadable(handle, String(error), error);
  }
  if (body.length !== handle.size) {
    throw unreadable(
      handle,
      `the file holds ${body.length} bytes where the body has ${handle.size}`,
    );
  }
  return body;
}

function unreadable(handle: types.BodyHandle, why: string, cause?: unknown): BodyUnavailableError {
  return new BodyUnavailableError(
    `could not read the response's body from \`${handle.path}\`: ${why}`,
    cause === undefined ? {} : { cause },
  );
}

/** The response a page's request got: a navigation's document, or what `page.request` answered. A 404 or a 500 is a response, a value. */
export class Response {
  readonly #host: HandleHost<types.PageId, CollieOperationTypes>;
  readonly #response: types.PageResponse;

  /** @internal The page's handle makes one; a program never constructs one. */
  constructor(host: HandleHost<types.PageId, CollieOperationTypes>, response: types.PageResponse) {
    this.#host = host;
    this.#response = response;
  }

  /** Its final URL. */
  url(): string {
    return this.#response.url;
  }

  /** Its HTTP status. */
  status(): number {
    return this.#response.status;
  }

  /** Its status text. */
  statusText(): string {
    return this.#response.statusText;
  }

  /** Whether its status is a success, from 200 to 299. It says nothing of the page: a login page answered with a 200 is `ok`. */
  ok(): boolean {
    return this.#response.status >= 200 && this.#response.status <= 299;
  }

  /** Its headers, their names lower-cased, as the browser received them. */
  headers(): Readonly<Record<string, string>> {
    return this.#response.headers;
  }

  /**
   * For a navigation's document, the encoding the browser decoded it with,
   * as `document.characterSet` names it, such as `Shift_JIS` (ADR 0119);
   * `null` for another response, and for a navigation that answered at
   * `commit`, before its document was decoded.
   */
  encoding(): string | null {
    return this.#response.encoding ?? null;
  }

  /** The engine's ID for its request's hop. */
  requestId(): string {
    return this.#response.requestId;
  }

  /** Whether a site's service worker supplied it, from its own cache or through its own fetch (ADR 0121). */
  fromServiceWorker(): boolean {
    return this.#response.fromServiceWorker;
  }

  /**
   * Reads the body of the response a page's request got, once the request finished, as a page body's handle: kept already for a response `page.wait_for_request_finished` answered, and otherwise read from the browser, on the session of the frame that loaded it. A body the browser no longer holds, or a redirect's, which has none, raises `body_unavailable`, whose fix names `page.wait_for_request_finished`; one that broke off raises `response_truncated`, never returned as whole. Past its `timeout`, `timeout.wait` when the call gives none, a request that has not ended fails it with `deadline_exceeded`; a page that ends under it raises what ended it, as a call on the page does.
   *
   * It answers its body's bytes, as the server sent them once their
   * content coding is undone, read through that handle. A file the engine
   * keeps it in that cannot be read also throws `BodyUnavailableError`.
   *
   * @experimental
   */
  async body(): Promise<Uint8Array> {
    warnExperimental(BODY);
    const handle = await this.#host.call(
      BODY,
      { pageId: this.#host.id, requestId: this.#response.requestId },
      { deadline: { key: "timeout.wait", value: undefined } },
    );
    return read(handle);
  }

  /**
   * Its body as text, decoded with the response's own encoding (ADR 0119):
   * a navigation's document with the one the browser decoded it with,
   * `encoding()`; any other response with its BOM, then its `Content-Type`
   * charset, then for HTML a `<meta>` as Chrome's prescan reads it (its
   * first 1,024 bytes always, then on while only tags allowed in the head
   * appear) or an XML declaration at its very start, for an XML type that
   * declaration, and UTF-8 last. A legacy charset such as `Shift_JIS` is
   * honoured; a BOM is left out, and a byte that does not decode becomes
   * U+FFFD. Unlike Playwright's `text()`, which decodes every body as
   * UTF-8, it honours the response's encoding, so a page that declares
   * another one reads as `page.content()` does, not as mojibake.
   *
   * It throws what `body()` throws: `ResponseTruncatedError` for a body
   * that broke off, never decoded as whole, and `BodyUnavailableError` for
   * one the browser no longer holds.
   */
  async text(): Promise<string> {
    const body = await this.body();
    return text(body, this.#response.encoding, this.#response.headers["content-type"]);
  }

  /**
   * Its body parsed as JSON, decoded as UTF-8 whatever its charset, as
   * JSON is, a leading BOM ignored.
   *
   * It throws what `body()` throws, `ResponseTruncatedError` for a body
   * that broke off included, and `ResponseNotJSONError` for a body that is
   * not JSON.
   */
  async json(): Promise<JSONValue> {
    const body = await this.body();
    try {
      return jsonOf(body);
    } catch (error) {
      const url = this.#response.url;
      throw new ResponseNotJSONError(
        `the body of ${url} is not the JSON asked for: ${String(error)}`,
        {
          url,
          cause: error,
        },
      );
    }
  }
}
