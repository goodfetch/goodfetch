// A crawl as its program declares it (ADR 0093, as ADRs 0099, 0102, 0103 and
// 0147 amend it): a `Crawl` value holds its Labels, the handler of each and
// its Datasets, and no connection, so it is declared at module scope and two
// Runs started from it share nothing. `Foxhound.startCrawl` starts it from
// its seeds; the engine then leases each work item to the handler of its
// Label, which reads its page and enqueues requests and pushes Records
// through its `CrawlContext`.
import type {
  JSONObject,
  JSONSchema,
  Locator,
  Page,
  SchemaInput,
  TypedSchema,
} from "@goodfetch/collie-client";
import { nonJSON, parse, wireSchema, type SchemaIssue } from "@goodfetch/collie-client/runtime";
import type {
  CrawlRequestEnqueueLinksParams,
  CrawlStartParams,
  DatasetDeclaration,
  Enqueued,
  EnqueuedLinks,
  HttpBody,
  LabelDeclaration,
  RequestSeed,
  Seed,
  SitemapSeed,
} from "./generated/index.ts";
import {
  LabelDuplicateError,
  LabelUnhandledError,
  LabelUnknownError,
  PayloadInvalidError,
} from "./generated/errors.ts";
import { settingsOnWire, type LabelSettings } from "./generated/settings.ts";

/** A JSON object the wire carries, such as a Payload or a Record. */
type WireObject = Readonly<Record<string, unknown>>;

/**
 * A Label's Payload schema (ADR 0082): a typed one whose input is an
 * object, Standard Schema with Standard JSON Schema such as Zod's, or a
 * JSON Schema written by hand.
 */
export type PayloadSchema = TypedSchema<object, unknown> | JSONSchema;

/** What a program sends as a Payload under `S`: its input type, or a JSON object for a hand-written one. */
export type PayloadInput<S extends PayloadSchema> = S extends TypedSchema
  ? SchemaInput<S>
  : JSONObject;

/** What `crawl.label` takes. */
export interface LabelOptions<S extends PayloadSchema | undefined = PayloadSchema | undefined> {
  /**
   * The schema of the Payload its requests carry (ADR 0082), which the
   * engine receives as JSON Schema; a Label without one carries none.
   */
  readonly payload?: S;
  /**
   * The Label's own settings over its Run's, nested by camelCase segment,
   * such as `{ fetch: { tier: "http" } }` (ADR 0093): a key a Label does not
   * take, a Run-wide one such as `politeness`, fails to compile.
   */
  readonly settings?: LabelSettings | undefined;
}

/** Each Label's Payload schema, set once when it is declared; a Label without one has none. */
const payloadSchemas = new WeakMap<Label<object | undefined>, PayloadSchema>();

/**
 * A kind of page a crawl handles (ADR 0093): each request names one, and
 * the handler attached to it handles its pages. `P` is the type of the
 * Payload its requests carry, `undefined` for a Label without one.
 */
export class Label<P extends object | undefined = undefined> {
  /** Tells a `Label<A>` from a `Label<B>` to the type checker; never set. */
  declare private readonly payload: P;
  /** Its name, unique in its Crawl. */
  readonly name: string;

  /** @internal `crawl.label` declares a Label; a program never constructs one. */
  constructor(name: string, schema: PayloadSchema | undefined) {
    this.name = name;
    if (schema !== undefined) payloadSchemas.set(this, schema);
  }
}

/**
 * What follows the Label in `enqueue`, `enqueueLinks` and `seed`: the
 * Payload, then the options, for a Label with a Payload, and the options
 * alone for one without (ADR 0082, G3).
 */
export type PayloadThen<P extends object | undefined, O> = [P] extends [undefined]
  ? [options?: O]
  : [payload: P, options?: O];

/** What `crawl.dataset` takes. */
export interface DatasetOptions<I extends object> {
  /** The fields whose values name a Record: a Record pushed again under the same key replaces it (ADR 0103). */
  readonly key?: readonly (keyof I & string)[] | undefined;
}

/**
 * A named collection of Records a crawl's handlers push (ADR 0103). `I` is
 * the type of each Record.
 */
export class Dataset<I extends object> {
  /** Tells a `Dataset<A>` from a `Dataset<B>` to the type checker; never set. */
  declare private readonly record: I;
  /** Its name, unique in its Crawl. */
  readonly name: string;

  /** @internal `crawl.dataset` declares a Dataset; a program never constructs one. */
  constructor(name: string) {
    this.name = name;
  }
}

/** What `context.enqueue` and `seed` take besides the URL, the Label and the Payload. */
export interface RequestOptions {
  /** Its HTTP method; `GET` when left out. */
  readonly method?: string | undefined;
  /** Its body, sent with its method. */
  readonly body?: HttpBody | null | undefined;
  /** Its place in the Frontier: a higher priority is fetched first. */
  readonly priority?: number | undefined;
  /** Whether it goes ahead of the requests of its priority. */
  readonly forefront?: boolean | undefined;
  /** What names it in place of its method, URL and body, so another of the same key is a duplicate. */
  readonly key?: string | null | undefined;
}

/** What `context.enqueueLinks` takes besides the locator, the Label and the Payload. */
export interface EnqueueLinksOptions {
  /** URL patterns, globs as `crawl.scope.include`'s, of which a link must match one when there are any; they only narrow the crawl's scope. */
  readonly include?: readonly string[] | undefined;
  /** URL patterns a link must match none of. */
  readonly exclude?: readonly string[] | undefined;
  /** At most this many links are added; each later one is not added, with `link_limit`. */
  readonly limit?: number | undefined;
  /** Their place in the Frontier, as `enqueue`'s: a higher priority is fetched first. */
  readonly priority?: number | undefined;
}

/**
 * What a handler's attempt at one work item holds (ADR 0093): the page the
 * engine lends it, and what it adds to the crawl, staged with the attempt.
 * Once the attempt ends, or its Run is closed, each call fails.
 */
export interface CrawlContext {
  /**
   * Already navigated. The handler runs once the request's navigation has
   * happened, so a route set on this page applies from its next navigation.
   * To keep a resource kind from loading on every request, set
   * `browser.block_resources`, which acts before each navigation (ADR 0117).
   *
   * The work item's page: a Collie `Page`, on every fetch tier (ADR 0093).
   */
  readonly page: Page;
  /**
   * Adds one request to a crawl from inside a handler's attempt, a handler
   * context's `enqueue`, and answers what the Frontier did with it: added,
   * with its crawl request's ID and its URL as parsed, or not added, with
   * its reason (`duplicate`, `out_of_scope`, `unsupported_scheme`,
   * `invalid_url`, `max_depth`, `max_requests`, `path_budget` or a URL
   * cap's name). A relative URL resolves against the document's base URL:
   * its `<base href>`, resolved against the URL the document was reached at
   * after its redirects, or that URL itself. A request is `duplicate` when
   * one of its identity was already admitted: its method, its URL's key
   * (the fragment, `utm_*`, `gclid`, `fbclid`, empty pairs and
   * `;jsessionid=` dropped, the parameters sorted) and its body's hash, or
   * the `key` the program declares in their place; it is still fetched at
   * its own URL. The request sits one level deeper than the one whose
   * attempt added it. An attempt whose lease no longer holds fails with
   * `attempt_ended`.
   *
   * The request goes under `label`, and its Payload comes after the Label
   * only when the Label has one (ADR 0082). A Label the crawl does not
   * declare, such as another crawl's, fails with `label_unknown`; a
   * Payload that is not JSON or that its Label's schema refuses fails with
   * `payload_invalid`, naming each issue's path, and nothing is sent. The
   * engine checks it again, and refuses one whose JSON is longer than
   * `crawl.payload.max_size` with `payload_invalid`.
   *
   * @experimental
   */
  enqueue<Q extends object | undefined>(
    url: string,
    label: Label<Q>,
    ...rest: PayloadThen<NoInfer<Q>, RequestOptions>
  ): Promise<Enqueued>;
  /**
   * Adds every link a locator matches on the page a handler's attempt was
   * given, a handler context's `enqueueLinks`, each a `GET` of its `href`
   * resolved against the document's base URL (its `<base href>`, resolved
   * against the URL the document was reached at after its redirects, or
   * that URL itself), its query percent-encoded in the document's encoding
   * as a browser encodes it, and answers each matched element in document
   * order: added, with its crawl request's ID and URL; not added, with its
   * reason, as `crawl_request.enqueue` answers; `no_link` for an element
   * without an `href`; or `skipped_by_payload` for one whose entry in
   * `payloads` is `null`. Every link takes `payload`, or its own entry of
   * `payloads`, the program's Payload function's answers for the elements
   * `crawl_request.links` read; a Payload that does not fit its Label fails
   * the call with `payload_invalid` and adds none. `limit` adds at most
   * that many links: once they are added, each later element whose link
   * the Frontier would judge is not added, with `link_limit`. `include` and
   * `exclude` only narrow the crawl's scope; a pattern the engine cannot
   * match fails with `pattern_unsupported`. A page no fetch tier serves
   * fails with `closed`, and an attempt whose lease no longer holds with
   * `attempt_ended`.
   *
   * The links go under `label`, each with the Payload that follows it when
   * the Label has one (ADR 0093). A Label the crawl does not declare fails
   * with `label_unknown`, and a Payload as `enqueue`'s refuses it with
   * `payload_invalid`, and nothing is sent.
   *
   * @experimental
   */
  enqueueLinks<Q extends object | undefined>(
    locator: Locator,
    label: Label<Q>,
    ...rest: PayloadThen<NoInfer<Q>, EnqueueLinksOptions>
  ): Promise<EnqueuedLinks>;
  /**
   * Pushes one Record or several, in one message, into a Dataset the crawl
   * declares from inside a handler's attempt, a handler context's `push`,
   * and answers once they are staged: they commit once with the attempt's
   * other outputs when the handler returns, and are discarded with them
   * when the attempt ends any other way. Each Record is checked first. A
   * push of no Record fails with `invalid_params`. A Dataset the crawl does
   * not declare fails with `dataset_unknown`, and none of the Records is
   * staged. A Record that is no JSON object, fails its Dataset's JSON
   * Schema or lacks one of its key's fields is not staged: its rejection is
   * staged instead, entering the Dataset's rejection table
   * (`run.rejected_records`) if the attempt commits, while the Records that
   * hold are staged and the attempt keeps what it staged before; the call
   * then fails with `record_invalid`, naming each such Record by its
   * position in the list and each of its issues by its path. An attempt
   * that already ended fails with `attempt_ended`.
   *
   * Each Record is typed from `dataset` alone (ADRs 0103 and 0147). A
   * handler that catches `record_invalid` keeps what it staged, this
   * push's other Records included. An empty list is never sent: it pushes
   * nothing and sends no message, so it does not fail.
   *
   * @experimental
   */
  push<I extends object>(
    dataset: Dataset<I>,
    records: NoInfer<I> | readonly NoInfer<I>[],
  ): Promise<void>;
}

/**
 * What handles one Label's pages: it runs on the event loop, and the
 * attempt ends once what it returns settles. A thrown error fails the
 * attempt; one that is not a Goodfetch error fails it with
 * `handler_failed`.
 */
export type Handler = (context: CrawlContext) => unknown;

/** What the engine leases a work item of one Label to. */
interface Declared {
  readonly label: Label<object | undefined>;
  readonly settings: LabelSettings | undefined;
  handler: Handler | undefined;
}

let declarationOf: (crawl: Crawl) => Pick<CrawlStartParams, "labels" | "datasets">;
let handlerIn: (crawl: Crawl, label: string) => Handler | undefined;
let declaredIn: (crawl: Crawl, label: Label<object | undefined>) => string;
let checkStartable: (crawl: Crawl, seeds: readonly Seed[]) => void;
let labelNamed: (crawl: Crawl, name: string) => Label<object | undefined> | undefined;

/**
 * A crawl's declaration (ADR 0093): its Labels, the handler of each and
 * its Datasets. It holds no connection and does no I/O.
 */
export class Crawl {
  static {
    declarationOf = (crawl) => crawl.#declaration();
    handlerIn = (crawl, label) => crawl.#labels.get(label)?.handler;
    declaredIn = (crawl, label) => crawl.#declared(label).label.name;
    checkStartable = (crawl, seeds) => crawl.#checkStartable(seeds);
    labelNamed = (crawl, name) => crawl.#labels.get(name)?.label;
  }

  readonly #labels = new Map<string, Declared>();
  readonly #datasets = new Map<string, DatasetDeclaration>();

  /**
   * Declares the Label `name`, whose requests carry Payloads of the
   * `payload` schema's input type, or none when it has no schema (ADR
   * 0082). A second Label of one name fails with `label_duplicate`.
   */
  label<S extends PayloadSchema>(
    name: string,
    options: LabelOptions<S> & { readonly payload: S },
  ): Label<PayloadInput<S>>;
  label(name: string, options?: LabelOptions<undefined>): Label;
  label(name: string, options: LabelOptions = {}): Label<object | undefined> {
    if (this.#labels.has(name)) {
      throw new LabelDuplicateError(`the Crawl already declares the Label ${name}`, {
        fix: "Declare each Label once, and use the Label the first `crawl.label` returned.",
        details: { label: name },
      });
    }
    const label = new Label<object | undefined>(name, options.payload);
    this.#labels.set(name, { label, settings: options.settings, handler: undefined });
    return label;
  }

  /**
   * Attaches `handler` to `label`: the engine leases each of its work items
   * to it. A Label this crawl does not declare, such as another crawl's,
   * fails with `label_unknown`, and a second handler for one Label with
   * `label_duplicate`.
   */
  handle(label: Label<object | undefined>, handler: Handler): void {
    const declared = this.#declared(label);
    if (declared.handler !== undefined) {
      throw new LabelDuplicateError(`the Label ${label.name} already has a handler`, {
        fix: "Attach one handler to each Label.",
        details: { label: label.name },
      });
    }
    declared.handler = handler;
  }

  /** Declares the Dataset `name`, whose Records are of type `I`. */
  dataset<I extends object>(name: string, options: DatasetOptions<I> = {}): Dataset<I> {
    this.#datasets.set(name, {
      name,
      ...(options.key === undefined ? {} : { key: [...options.key] }),
    });
    return new Dataset<I>(name);
  }

  /** The declaration of `label`, which must be the very Label this crawl's `crawl.label` returned. */
  #declared(label: Label<object | undefined>): Declared {
    const declared = this.#labels.get(label.name);
    if (declared?.label !== label) throw this.#unknown(label.name);
    return declared;
  }

  /**
   * Fails when a Label has no handler, or a seed or a sitemap seed's rule
   * names a Label this crawl does not declare: by name, since a seed
   * carries its Label's name only.
   */
  #checkStartable(seeds: readonly Seed[]): void {
    for (const [name, { handler }] of this.#labels) {
      if (handler === undefined) {
        throw new LabelUnhandledError(`the Crawl attaches no handler to the Label ${name}`, {
          fix: "Attach one with `crawl.handle(label, handler)`, or stop declaring the Label.",
          details: { label: name },
        });
      }
    }
    const unknown = seeds.flatMap(labelsOf).find((label) => !this.#labels.has(label));
    if (unknown !== undefined) throw this.#unknown(unknown);
  }

  #unknown(name: string): LabelUnknownError {
    return new LabelUnknownError(`the Crawl does not declare the Label ${name}`, {
      fix: "Use a Label this crawl's `crawl.label` returned, or declare it on this crawl.",
      details: { label: name, declared: [...this.#labels.keys()] },
    });
  }

  #declaration(): Pick<CrawlStartParams, "labels" | "datasets"> {
    const labels: Record<string, LabelDeclaration> = {};
    for (const [name, { label, settings }] of this.#labels) {
      const wire = settingsOnWire(settings);
      const schema = payloadSchemas.get(label);
      labels[name] = {
        ...(schema === undefined ? {} : { payload: wireSchema(schema) }),
        ...(wire === undefined ? {} : { settings: wire }),
      };
    }
    const datasets = [...this.#datasets.values()];
    return { labels, ...(datasets.length === 0 ? {} : { datasets }) };
  }
}

/** The Labels and Datasets `crawl.start` declares for `crawl`. */
export function crawlDeclaration(crawl: Crawl): Pick<CrawlStartParams, "labels" | "datasets"> {
  return declarationOf(crawl);
}

/**
 * Fails, before anything is sent, when `crawl` declares a Label without a
 * handler (`label_unhandled`) or a seed names a Label it does not declare by name
 * (`label_unknown`).
 */
export function checkCrawlStartable(crawl: Crawl, seeds: readonly Seed[]): void {
  checkStartable(crawl, seeds);
}

/**
 * Fails with `payload_invalid`, before anything is sent, when a seed's
 * Payload is refused by its Label's schema's own parse; `seeds` name only
 * Labels `crawl` declares, as `checkCrawlStartable` checked. A sitemap
 * seed carries no Payload.
 */
export async function checkSeedPayloads(crawl: Crawl, seeds: readonly Seed[]): Promise<void> {
  for (const sent of seeds) {
    if (isSitemap(sent)) continue;
    const declared = labelNamed(crawl, sent.label);
    if (declared !== undefined) await checkPayload(declared, sent.payload);
  }
}

/** Whether `sent` is a sitemap seed rather than a request. */
function isSitemap(sent: Seed): sent is SitemapSeed {
  return "sitemap" in sent;
}

/** The names of the Labels `sent` gives its requests: a request's own, or each of a sitemap seed's rules'. */
function labelsOf(sent: Seed): string[] {
  return isSitemap(sent) ? sent.rules.map(({ label }) => label) : [sent.label];
}

/** The name of `label`, which `crawl` must declare; another crawl's Label fails with `label_unknown`. */
export function declaredLabel(crawl: Crawl, label: Label<object | undefined>): string {
  return declaredIn(crawl, label);
}

/** The handler attached to the Label named `label` in `crawl`, if any. */
export function handlerOf(crawl: Crawl, label: string): Handler | undefined {
  return handlerIn(crawl, label);
}

/**
 * A seed request of a crawl: `url`, handled under `label`, carrying the
 * Payload that follows it when the Label has one (ADR 0082). A Payload
 * that is not JSON fails with `payload_invalid` here, and one its Label's
 * schema refuses fails `startCrawl` with it, before anything is sent.
 */
export function seed<P extends object | undefined>(
  url: string,
  label: Label<P>,
  ...rest: PayloadThen<NoInfer<P>, RequestOptions>
): RequestSeed {
  const { payload, options = {} } = payloadAndOptions(label, rest);
  return { url, label: label.name, ...payloadOf(label, payload), ...requestOptions(options) };
}

/**
 * A sitemap seed of a crawl: the entries of `url`'s sitemaps (ADR 0093).
 * `url` names a sitemap, or a site's robots.txt, whose `Sitemap:` lines
 * are read, and nested sitemap indexes are followed. Each `[pattern,
 * label]` of `rules`, in order, is a glob as `crawl.scope.include` takes
 * one and the Label it gives the entries it matches: the first that
 * matches an entry picks its Label, and an entry no rule matches is not
 * added, with `no_rule`. A sitemap entry carries no Payload, so a rule's
 * Label takes none. The engine reads the sitemap once the Run is running.
 */
export function sitemap(
  url: string,
  rules: Iterable<readonly [pattern: string, label: Label]>,
): SitemapSeed {
  return {
    sitemap: url,
    rules: Array.from(rules, ([pattern, label]) => ({ pattern, label: label.name })),
  };
}

/**
 * What follows `label` in a call that takes `PayloadThen`: its Payload
 * and options for a Label with a Payload, its options alone for one
 * without.
 */
export function payloadAndOptions<P extends object | undefined, O>(
  label: Label<P>,
  rest: PayloadThen<P, O>,
): { readonly payload: unknown; readonly options: O | undefined } {
  const given: readonly unknown[] = rest;
  const [first, second] = given;
  const hasPayload = payloadSchemas.has(label);
  const options = hasPayload ? second : first;
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `PayloadThen` types what follows the Label as its options, after the Payload when the Label has one.
  return { payload: hasPayload ? first : undefined, options: options as O | undefined };
}

/**
 * The wire's `payload` field for `payload`, sent under `label`: none when
 * the Label has none. A value that is not JSON fails with
 * `payload_invalid`, naming its path.
 */
export function payloadOf(
  label: Label<object | undefined>,
  payload: unknown,
): { readonly payload?: WireObject } {
  if (!payloadSchemas.has(label)) return {};
  const refusal = nonJSON(payload);
  if (refusal !== undefined) {
    throw payloadRefused(label.name, [
      { path: refusal.path, message: `${refusal.found} is not JSON` },
    ]);
  }
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `nonJSON` found nothing that is not JSON, and `PayloadThen` types a Payload as an object.
  return { payload: payload as WireObject };
}

/**
 * Fails with `payload_invalid` when `payload`, sent under `label`, is
 * refused by the Label's schema's own parse (ADR 0082), naming each issue.
 */
export async function checkPayload(
  label: Label<object | undefined>,
  payload: WireObject | undefined,
): Promise<void> {
  const schema = payloadSchemas.get(label);
  if (schema === undefined) return;
  // A declared schema reads a missing Payload as `null`, as the engine does.
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `payloadOf` checked that a Payload is JSON.
  const parsed = await parse(schema, (payload ?? null) as JSONObject | null);
  if ("issues" in parsed) throw payloadRefused(label.name, parsed.issues);
}

/** The `payload_invalid` of a Payload for the Label `label` refused for `issues`. */
function payloadRefused(label: string, issues: readonly SchemaIssue[]): PayloadInvalidError {
  const named = issues.map(({ path, message }) => `${path || "its root"}: ${message}`);
  return new PayloadInvalidError(
    `the Payload for the Label ${label} was not sent: ${named.join("; ")}`,
    {
      fix: "Send a Payload its Label's schema accepts, made of JSON values only: a string, a finite number, a boolean, null, an array or a plain object.",
      details: { label, issues: issues.map(({ path, message }) => ({ path, message })) },
    },
  );
}

/** `value`'s own enumerable fields as the JSON object the wire carries. */
export function wireObject(value: object): WireObject {
  return Object.fromEntries(Object.entries(value));
}

/** The fields of `context.enqueueLinks`'s `options` the wire carries, those left out omitted. */
export function enqueueLinksOptions({
  include,
  exclude,
  limit,
  priority,
}: EnqueueLinksOptions): Pick<
  CrawlRequestEnqueueLinksParams,
  "include" | "exclude" | "limit" | "priority"
> {
  return {
    ...(include === undefined ? {} : { include: [...include] }),
    ...(exclude === undefined ? {} : { exclude: [...exclude] }),
    ...(limit === undefined ? {} : { limit }),
    ...(priority === undefined ? {} : { priority }),
  };
}

/** The fields of `options` the wire carries, those left out omitted. */
export function requestOptions({
  method,
  body,
  priority,
  forefront,
  key,
}: RequestOptions): Pick<RequestSeed, "method" | "body" | "priority" | "forefront" | "key"> {
  return {
    ...(method === undefined ? {} : { method }),
    ...(body === undefined ? {} : { body }),
    ...(priority === undefined ? {} : { priority }),
    ...(forefront === undefined ? {} : { forefront }),
    ...(key === undefined ? {} : { key }),
  };
}
