// A crawl as its program declares it (ADR 0093, as ADRs 0099, 0102, 0103 and
// 0147 amend it): a `Crawl` value holds its Labels, the handler of each and
// its Datasets, and no connection, so it is declared at module scope and two
// Runs started from it share nothing. `Foxhound.startCrawl` starts it from
// its seeds; the engine then leases each work item to the handler of its
// Label, which reads its page and enqueues requests and pushes Records
// through its `CrawlContext`.
import type { Page } from "@goodfetch/collie-client";
import type {
  CrawlStartParams,
  DatasetDeclaration,
  Enqueued,
  HttpBody,
  LabelDeclaration,
  RequestSeed,
} from "./generated/index.ts";

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
   * reason. A relative URL resolves against the page's base URL.
   */
  enqueue<Q extends object | undefined>(
    url: string,
    label: Label<Q>,
    payload?: Q,
    options?: RequestOptions,
  ): Promise<Enqueued>;
  /** Pushes one Record into `dataset`, committed with the attempt (ADR 0103). */
  push<I extends object>(dataset: Dataset<I>, record: I): Promise<void>;
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

/**
 * A crawl's declaration (ADR 0093): its Labels, the handler of each and
 * its Datasets. It holds no connection and does no I/O.
 */
export class Crawl {
  static {
    declarationOf = (crawl) => crawl.#declaration();
    handlerIn = (crawl, label) => crawl.#labels.get(label)?.handler;
  }

  readonly #labels = new Map<string, Declared>();
  readonly #datasets = new Map<string, DatasetDeclaration>();

  /** Declares the Label `name`, whose requests carry Payloads of type `P`. */
  label<P extends object | undefined = undefined>(
    name: string,
    options: LabelOptions = {},
  ): Label<P> {
    const label = new Label<P>(name);
    this.#labels.set(name, { label, settings: options.settings, handler: undefined });
    return label;
  }

  /** Attaches `handler` to `label`: the engine leases each of its work items to it. */
  handle(label: Label<object | undefined>, handler: Handler): void {
    const declared = this.#labels.get(label.name) ?? {
      label,
      settings: undefined,
      handler: undefined,
    };
    declared.handler = handler;
    this.#labels.set(label.name, declared);
  }

  /** Declares the Dataset `name`, whose Records are of type `I`. */
  dataset<I extends object>(name: string, options: DatasetOptions<I> = {}): Dataset<I> {
    this.#datasets.set(name, {
      name,
      ...(options.key === undefined ? {} : { key: [...options.key] }),
    });
    return new Dataset<I>(name);
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
