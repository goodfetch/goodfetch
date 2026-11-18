// A crawl as its program declares it (ADR 0093, as ADRs 0099, 0102, 0103 and
// 0147 amend it): a `Crawl` value holds its Labels, the handler of each and
// its Datasets, and no connection, so it is declared at module scope and two
// Runs started from it share nothing. `Foxhound.startCrawl` starts it from
// its seeds; the engine then leases each work item to the handler of its
// Label, which reads its page and enqueues requests and pushes Records
// through its `CrawlContext`.
import type { Locator, Page } from "@goodfetch/collie-client";
import type {
  CrawlRequestEnqueueLinksParams,
  CrawlStartParams,
  DatasetDeclaration,
  Enqueued,
  EnqueuedLinks,
  HttpBody,
  LabelDeclaration,
  RequestSeed,
} from "./generated/index.ts";
import { LabelDuplicateError, LabelUnhandledError, LabelUnknownError } from "./generated/errors.ts";

/** A JSON object the wire carries, such as a Payload or a Record. */
type WireObject = Readonly<Record<string, unknown>>;

/** What `crawl.label` takes. */
export interface LabelOptions {
  /** The Label's own settings, nested by key, such as `fetch.tier` (ADR 0093). */
  readonly settings?: LabelDeclaration["settings"] | undefined;
}

/**
 * A kind of page a crawl handles (ADR 0093): each request names one, and
 * the handler attached to it handles its pages. `P` is the type of the
 * Payload its requests carry.
 */
export class Label<P extends object | undefined = undefined> {
  /** Tells a `Label<A>` from a `Label<B>` to the type checker; never set. */
  declare private readonly payload: P;
  /** Its name, unique in its Crawl. */
  readonly name: string;

  /** @internal `crawl.label` declares a Label; a program never constructs one. */
  constructor(name: string) {
    this.name = name;
  }
}

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
  /** The work item's page: a Collie `Page`, on every fetch tier (ADR 0093). */
  readonly page: Page;
  /**
   * Adds a request to the crawl, under `label`, and answers what the
   * Frontier did with it: added, with its ID, or not added, with its
   * reason. A relative URL resolves against the page's base URL. A
   * Label the crawl does not declare, such as another crawl's, fails with
   * `label_unknown`, and nothing is sent.
   */
  enqueue<Q extends object | undefined>(
    url: string,
    label: Label<Q>,
    payload?: Q,
    options?: RequestOptions,
  ): Promise<Enqueued>;
  /**
   * Adds every link `locator` matches on the attempt's page, under
   * `label`, each with `payload`, and answers what became of each matched
   * element in document order: added, with its ID, not added, with its
   * reason, or with no link. Each `href` resolves against the document's
   * base URL (ADR 0093). A Label the crawl does not declare fails with
   * `label_unknown`, and nothing is sent.
   */
  enqueueLinks<Q extends object | undefined>(
    locator: Locator,
    label: Label<Q>,
    payload?: Q,
    options?: EnqueueLinksOptions,
  ): Promise<EnqueuedLinks>;
  /**
   * Pushes one Record, or a list of Records in one message, into
   * `dataset`, each typed from the Dataset alone and committed with the
   * attempt (ADRs 0103 and 0147). Each Record is checked: those that hold
   * are staged, and when any does not, the push fails with
   * `record_invalid`, naming each such Record by its position in the list
   * and each issue by its path. A handler that catches it keeps what it
   * staged, this push's other Records included. An empty list pushes
   * nothing and sends no message.
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
  readonly settings: LabelDeclaration["settings"] | undefined;
  handler: Handler | undefined;
}

let declarationOf: (crawl: Crawl) => Pick<CrawlStartParams, "labels" | "datasets">;
let handlerIn: (crawl: Crawl, label: string) => Handler | undefined;
let declaredIn: (crawl: Crawl, label: Label<object | undefined>) => string;
let checkStartable: (crawl: Crawl, seeds: readonly RequestSeed[]) => void;

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
  }

  readonly #labels = new Map<string, Declared>();
  readonly #datasets = new Map<string, DatasetDeclaration>();

  /**
   * Declares the Label `name`, whose requests carry Payloads of type `P`.
   * A second Label of one name fails with `label_duplicate`.
   */
  label<P extends object | undefined = undefined>(
    name: string,
    options: LabelOptions = {},
  ): Label<P> {
    if (this.#labels.has(name)) {
      throw new LabelDuplicateError(`the Crawl already declares the Label ${name}`, {
        fix: "Declare each Label once, and use the Label the first `crawl.label` returned.",
        details: { label: name },
      });
    }
    const label = new Label<P>(name);
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
   * Fails when a Label has no handler, or a seed names a Label this crawl
   * does not declare: by name, since a seed carries its Label's name only.
   */
  #checkStartable(seeds: readonly RequestSeed[]): void {
    for (const [name, { handler }] of this.#labels) {
      if (handler === undefined) {
        throw new LabelUnhandledError(`the Crawl attaches no handler to the Label ${name}`, {
          fix: "Attach one with `crawl.handle(label, handler)`, or stop declaring the Label.",
          details: { label: name },
        });
      }
    }
    const unknown = seeds.find(({ label }) => !this.#labels.has(label));
    if (unknown !== undefined) throw this.#unknown(unknown.label);
  }

  #unknown(name: string): LabelUnknownError {
    return new LabelUnknownError(`the Crawl does not declare the Label ${name}`, {
      fix: "Use a Label this crawl's `crawl.label` returned, or declare it on this crawl.",
      details: { label: name, declared: [...this.#labels.keys()] },
    });
  }

  #declaration(): Pick<CrawlStartParams, "labels" | "datasets"> {
    const labels: Record<string, LabelDeclaration> = {};
    for (const [name, { settings }] of this.#labels) {
      labels[name] = settings === undefined ? {} : { settings };
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
export function checkCrawlStartable(crawl: Crawl, seeds: readonly RequestSeed[]): void {
  checkStartable(crawl, seeds);
}

/** The name of `label`, which `crawl` must declare; another crawl's Label fails with `label_unknown`. */
export function declaredLabel(crawl: Crawl, label: Label<object | undefined>): string {
  return declaredIn(crawl, label);
}

/** The handler attached to the Label named `label` in `crawl`, if any. */
export function handlerOf(crawl: Crawl, label: string): Handler | undefined {
  return handlerIn(crawl, label);
}

/** A seed request of a crawl: `url`, handled under `label`, carrying `payload`. */
export function seed<P extends object | undefined>(
  url: string,
  label: Label<P>,
  payload?: P,
  options: RequestOptions = {},
): RequestSeed {
  return { url, label: label.name, ...payloadOf(payload), ...requestOptions(options) };
}

/** The wire's `payload` field for `payload`: none when it is left out. */
export function payloadOf(payload: object | undefined): { readonly payload?: WireObject } {
  return payload === undefined ? {} : { payload: wireObject(payload) };
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
