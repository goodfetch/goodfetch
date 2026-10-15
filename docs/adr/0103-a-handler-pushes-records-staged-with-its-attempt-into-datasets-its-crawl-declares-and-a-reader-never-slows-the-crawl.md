# A handler pushes Records staged with its attempt into Datasets its crawl declares, and a reader never slows the crawl

A crawl's handler pushes each Record through its context into a Dataset its crawl declares. The Record is staged with the attempt and commits once with it, numbered in its Dataset. A program reads committed Records back as a stream that starts at the Dataset's first Record and never slows the crawl. Sinks are settings, an Export is a file the program saves wherever the engine runs, and every Run keeps a Key-value store.

**What this answers.**
- Pain 11 of the [research on competitors' APIs](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md) (duplicate or partial results on retry), and its open questions 13 (push or pull) and 14.
- The [crawl frameworks note](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/crawl-frameworks.md) §4.2 and §6.14.
- What ADRs 0079, 0081, 0082 and 0093 handed over: dropped and rejected Records, where the Records sequence starts, how a Dataset is declared and the code of a rejected Record, and pushing from the handler.

**Competitors, re-read at tagged releases on 2026-10-15.**
- **Crawlee 3.18.2.** The classic crawler writes `context.pushData` at once, so a handler that throws keeps what it pushed ([basic-crawler.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L1668)).
- **Crawlee v4 and Crawlee Python 1.10.4** stage a handler's storage writes until it succeeds. Crawlee v4 still writes queue additions through, and commits "at-least-once" ([upgrade guide](https://github.com/apify/crawlee/blob/8cb2eb61a0356a96205c08e02ed49cadc97081e5/docs/upgrading/upgrading-v4/upgrading-v4.md#L850-L860), [_basic_crawler.py](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/crawlers/_basic/_basic_crawler.py#L1509)).
- **Scrapy 2.19.0** keeps items yielded before a callback fails, and has no API to iterate items from a script ([scraper.py](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scraper.py#L399-L438)).
- **Spider 2.52.2's `subscribe()`** loses pages when its reader lags ([website.rs](https://github.com/spider-rs/spider/blob/v2.52.2/spider/src/website.rs#L14296-L14310)).
- **Scrapling 0.4.15's `stream()`** slows the crawl through a buffer of 100 ([engine.py](https://github.com/D4Vinci/Scrapling/blob/v0.4.15/scrapling/spiders/engine.py#L460)).
- **Crawl4AI 0.9.5's `arun_many()`** returns a list or a generator, depending on `config.stream` ([async_webcrawler.py](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_webcrawler.py#L1101-L1137)).

Decided in [Crawl output in code](https://github.com/goodfetch/automan/issues/479), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461). Every answer was decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/automan/issues/479#issuecomment-6181693852). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). Datasets, Records, Exports and Sinks are Foxhound's (ADR 0084); the Key-value store is every Run's.

## Decision

### Writing: one way to push, one way to read

- **Two tasks, each with one way (S2).**
  - A handler pushes Records into a Dataset in run state, so they checkpoint, resume, reach Sinks once and show in the Console (ADRs 0002, 0003, 0008).
  - A program reads committed Records back as a stream.
  - Crawlee's `pushData` and `getData` split the same way.
- **`ctx.push(dataset, record)`** (`ctx.push(books, Book(...))` in Python, `ctx.push(&books, &book).await?` in Rust).
  - It is on the context, which is the attempt (ADR 0093), as Crawlee's `context.pushData` is (S3).
  - The destination comes first, as in `keyValueStore.set(key, value)`.
  - TypeScript types the Record from the Dataset alone (`NoInfer`), so a Record literal missing a field never widens the type.
- **Records come only from handlers.** There is no `run.push`, so every Record's Provenance names the crawl request that produced it. A program's own state outside a handler goes to the Key-value store.
- **Not on a handle, nor on the Run.**
  - `books.push(record)` on a handle declared at import would have to find its attempt from hidden context, such as `AsyncLocalStorage` or `contextvars` (G2).
  - A `push` on the Run would exist only on `ctx.run`'s attempt-bound view (ADR 0093), never on the program's own Run.

### Datasets are declared on the crawl

- **The declaration.** `crawl.dataset(name, { schema, key })`, `dataset(name, schema=…, key=…)` in Python, and `crawl.dataset::<T>(name)` or `crawl.dataset_with(name, schema, key)` in Rust.
  - It returns a typed `Dataset<I, O>` handle, with no I/O, as `crawl.label()` does (ADR 0093).
  - With no schema, it takes any JSON object (ADR 0082).
  - The engine knows every Dataset when the Run starts, so a Sink naming one is checked before the first Record (ADR 0026).
- **No default Dataset.** Every push names a declared handle, so a misspelled Dataset is a type error (G3, T). Crawlee's `pushData(data, 'bokks')` silently opens a second Dataset.
- **Mistakes, caught at once:**
  - a second Dataset of one name raises `dataset_duplicate` when it is declared;
  - a push to a handle that another crawl declared raises `dataset_unknown`.
- **Another program reads a Run's Records** with the handle that the crawl's module declares, since declaring does no I/O. The catalog's raw call, by the Dataset's name, is the way down (G8).

### What a push guarantees

- **Staged with the attempt (G1).**
  - `ctx.push` checks the Record and stages it. It resolves once the Record is staged, not once it is committed.
  - The attempt's Records commit together with its enqueues and Key-value store writes, under a valid lease and run epoch, once the handler returns (ADR 0003).
  - An attempt that fails, retries, escalates, skips or loses its lease discards them. A retried request therefore commits its Records once, and a failed one leaves none.
  - There is no write-through setting: Crawlee 3's write-through is the behaviour pain 11 complains of.
- **Numbering.** A committed Record gets the next number of its Dataset, in commit order, with no gaps (ADR 0008).
- **Sample comparison.** ADR 0020's comparison runs a handler on two tiers and compares the Records each attempt staged. Only the chosen attempt commits.
- **A Record that fails raises from `push` (G1, G3, G6).**
  - The code is `record_invalid`, naming the Dataset and each issue's path. It covers:
    - a Record that fails its schema, natively in the SDK or against the JSON Schema in the engine (ADR 0082);
    - a value that is not JSON;
    - a Record that lacks its Dataset's key fields.
  - The failing Record is never staged. A handler that catches the error keeps its attempt and its other Records.
  - If the error escapes, the request ends `failed` with `record_invalid` (ADR 0079), which is not retryable, so the request is not retried.
  - Diverting such a Record silently, or a setting that chooses, is rejected: a request would read `succeeded` while its data was missing, and a setting that turns an error into a value is the raise switch ADR 0079 forbids.
- **Dropped and rejected Records (ADR 0033's "Record before commit" Hook stage).**
  - A Hook's answer never fails the request.
  - A transformed Record is checked against its schema again, and is rejected if it no longer fits.
  - A `succeeded` outcome carries `records`: for each Dataset, how many Records were committed, dropped and rejected. A request that succeeded with nothing committed says why (G1).
  - `run.rejectedRecords(dataset)` reads the Dataset's rejection table. Each entry is the Record as JSON, never parsed by the schema it failed (G3), with the reason, the Hook, the request's ID and its URL.
  - Dropped Records are counted by reason in the run's report, never stored.
  - A Sink's dead letters stay ADR 0026's.
- **Duplicates.**
  - **Retries:** staging prevents them.
  - **Deliveries:** the Record's number, with ADR 0008's delivery ID and run epoch, prevents them.
  - **The same thing collected twice** (a book in two categories, or in tomorrow's run): the Dataset's declared `key`.
    - The key lists top-level fields that the schema requires. In TypeScript they are typed as the schema's field names, and a field the schema does not require raises `dataset_key_invalid` when the Dataset is declared.
    - The Dataset stays append-only. The key is the default key of every Sink's keyed write modes (ADR 0026), and identifies a Record against a Baseline (ADR 0036).
    - Dropping a duplicate before commit is a Hook's job, with its reason counted.
    - A Dataset with no key has no keyed write mode.

### Reading Records

- **The call.** `run.records(dataset, { after })` on a crawl's Run: `records(books, after=…)` in Python, and `records(&books)` or `records_after(&books, n)` in Rust.
  - It is an iterator, subscribed when called (ADR 0081).
  - It yields `DatasetRecord<O>`: `number`, `value` (the schema's output type, built by its native parse, ADR 0082), `requestId`, `url` and `committedAt`. `Record` alone would shadow TypeScript's built-in `Record` type.
- **Where it starts.** At the Dataset's first Record, or after `after`, which resumes a reader from the last number it handled (ADR 0008). Starting at the moment of the call would silently miss the Records committed before it (G1); Spider's `subscribe()` does.
- **When it ends.** It follows the Run live, and ends once the Run has ended and every Record has been read. On a Run that has already ended, it reads to the last Record.
- **A failed parse.** A Record that its native parse rejects ends the iteration with `record_invalid`, carrying its number, so the reader can resume after it.
- **Only a crawl's Run has Records** (ADR 0084), so `records` is on `CrawlRun`, not on every `Run`.
- **A reader never slows the crawl (G4, G1).**
  - Records are committed to run state before anyone reads them, so a reader is a cursor over them. The engine sends what the reader's credit allows, and the crawl never waits for it.
  - Records are lossless because they are durable. Spider's channel loses pages and Scrapling's stream slows the crawl; a paused debugger, a slow call per Record or a forgotten iterator would otherwise hold every lease.
  - Handler requests, which are the crawl's work, still slow their producer (ADR 0024).

### Exports

- **The call.** `run.export(dataset, { format, compression })` writes an Export of the Records committed so far (ADR 0026).
- **What it returns.** An `Export` with its format, its Record count, its last record number, its size and its SHA-256, and `saveAs(path)` and `stream()`: Playwright's `Download` names for a file the engine holds (S3).
- **Wherever the engine runs.** A path on the engine's own machine would break on a Daemon or in Goodfetch Cloud (ADR 0001; the research's "works locally but not on the Apify Platform").
- **The format is named, never inferred** from a file name, as Crawlee's `exportData` infers it, so a typo is a type error (G3). Each format's options are `export.*` settings, named by area 6.

### The Key-value store

- **On every Run, Collie's included:** `run.keyValueStore` (`run.key_value_store` in Python, `run.key_value_store()` in Rust).
- **JSON values.**
  - `get(key)` returns JSON, or null when the key is absent: a question's answer (ADR 0079).
  - `get(key, schema)` returns the schema's output type, built by its native parse.
  - `set(key, value)` takes JSON, or in Python a Pydantic model.
- **Bytes.** `getBytes(key)` returns `{ content, contentType }` or null, and `setBytes(key, content, contentType)` writes them.
- **Removing and listing.** `delete(key)` returns whether the key existed. `keys({ prefix })` is an iterator.
- **One code for a wrong value.** A value that is not JSON, a value that fails its schema, or bytes read with `get` raise `value_invalid`, naming the key, and naming `getBytes` in its `fix`.
- **Map-like names (S1).** Crawlee's `getValue` returns JSON or a `Buffer` depending on the content type, one method with two return types (S2), so its names are not taken (S3 applies only where the meaning matches).
- **In a handler, the store is reached through `ctx.run`.** ADR 0093 binds `ctx.run` to its attempt, so its writes stage with the attempt and its reads see the attempt's own writes first, as `ctx.run.addSeed` stages.
  - A cursor, or a blob that a Record points to, then commits with the work it belongs to.
  - The program's own `run` value, captured by the handler, writes at once, for state that must outlive a failed attempt.
- **No `useState`.**
  - Crawlee's `useState` is one live object, saved every 60 s, that even Crawlee v4 leaves outside its transactional writes. It is state the program does not hold (G2), and two concurrent handlers lose each other's updates.
  - `get(key, schema)` and `set` cover it.

### Sinks in code

- **Sinks are settings.** `sinks.<name>.*` is one family per Sink: its type, the Datasets it receives, its destination (a Secret reference), its write mode and its key, defaulting to its Datasets' keys.
  - It is set on a connection or a crawl's Run, from code, the project file, the CLI or a Recipe alike (ADRs 0028, 0083), because where data goes differs by deployment while the crawl's logic does not.
  - It is Run-wide, so a Label refuses it (ADR 0093). A Collie Run refuses it too, since it has no Datasets.
  - Each Sink type's keys are area 6's spec.
- **Checked at start.** A Sink naming a Dataset that the crawl does not declare is refused with `setting_invalid`, naming the closest Dataset (G3).
- **A Sink written in the program** (ADR 0026's SDK-handler form) is a Plugin in code. [Hooks and injectable components in the SDK API](https://github.com/goodfetch/automan/issues/481) decides it.
- **A reader is not a Sink.** `run.records()` keeps no delivery state in the engine.

### Raw responses

Nothing is added to the SDK API.
- `cache.mode: "replay"` reruns every handler on stored responses without fetching (ADR 0046).
- `ctx.response` keeps its body (ADR 0093).
- WARC output (ADR 0037's CO-30) stays a Run setting whose keys area 6 names.

### Error codes

`dataset_duplicate`, `dataset_unknown`, `dataset_key_invalid`, `record_invalid` and `value_invalid` join `goodfetch-protocol`'s list in the `program` group (ADR 0079). None is retryable: the same call fails the same way until the program or its data changes.

## Considered Options

- **Pull only** (Spider, Crawl4AI): rejected. Handlers already run in the program, and Records outside run state would miss Checkpoints, Sinks and the Console.
- **Push only:** rejected. A program that wants its data in hand (a crawl in a service) would need a Sink pointing back at itself.
- **A Python generator yielding Records** (Scrapy's `yield item`): rejected. It would be a second way to push, unavailable in TypeScript and Rust, and it still needs the Dataset named.
- **A default Dataset:** rejected. It would be a nameless second way to push, and a Sink could not name it.
- **Write-through pushes,** or a setting for them: rejected. A failed attempt would leave partial Records, which is pain 11.
- **Deduplicating by key inside the Dataset:** rejected. An append-only Dataset would silently lose a Record (G1).
- **A reader that slows the crawl** (Scrapling, ADR 0024 as it stood): rejected. The Records are already durable.
- **Inferring the Export format from the file name** (Crawlee): rejected by G3.
- **A path on the engine's machine for Exports:** rejected. It breaks remote engines.
- **Sink objects passed to the start verb,** or Sinks on the `Crawl`: rejected. The CLI, Recipes and the project file could not set them, and a deployment could not change them.

## Consequences

- **Amends ADR 0024.** Records read through a crawl's Run come from run state as a cursor and never slow the producer; handler requests still do. A note sits under its event-delivery bullet.
- **Amends ADR 0081.** An unread Records sequence does not slow its Run, and `records` is on a crawl's Run. A note sits under its Iterators bullet.
- **Amends ADR 0079.** A `succeeded` crawl request outcome counts its Records per Dataset: committed, dropped and rejected. A note sits under its outcomes bullet.
- **Amends ADR 0033.** A Record that a Hook transformed is checked against its schema again, and a program reads the rejection table with `run.rejectedRecords()`. A note sits under its Hook stage table.
- **Amends ADR 0026.** The user-declared fields of the keyed write modes default to the Dataset's `key`, and Sinks are configured as `sinks.<name>.*` settings. A note sits under its write modes.
- **Amends ADR 0093**, once merged. Records are pushed with `ctx.push`, not through `ctx.run`.
- **ADR 0037's matrix.**
  - CO-25, CO-26, CO-27, CO-29 and RT-09 cite this ADR; RT-09's `useState` is declined.
  - Areas 3, 6 and 11 list it.
- **`GLOSSARY.md`.** Dataset and Record are sharpened: a Dataset is declared by its crawl and keyed by the fields that identify a Record, and a Record is committed with the attempt that pushed it.
- **Who builds what.**
  - **Area 3 (Engine runtime):** staging Records and Key-value store writes with the attempt, per-Dataset numbering, and the reader's cursor.
  - **Area 6 (Collection):** Datasets with keys and their checks, the rejection table and its counts, Exports in code, the Key-value store's operations, the `sinks.*` and `export.*` keys, and WARC's keys.
  - **Area 11 (Client surfaces):** `crawl.dataset`, `ctx.push`, `records`, `rejectedRecords`, `export`, the Key-value store, and each language's error classes.
- **Other tickets of the map.**
  - [The Run in code](https://github.com/goodfetch/automan/issues/478) reads outcomes with their Record counts, each Sink's delivery status, and opens a past Run by ID.
  - [Hooks and injectable components in the SDK API](https://github.com/goodfetch/automan/issues/481) shapes the "Record before commit" Hook and SDK-handler Sinks.
  - [Testing a user's crawl or automation](https://github.com/goodfetch/automan/issues/482) records what a test handler pushed.
- **The `prototype/sdk-api` sketch** holds these signatures in its `output` files for each language, with their snippets.
