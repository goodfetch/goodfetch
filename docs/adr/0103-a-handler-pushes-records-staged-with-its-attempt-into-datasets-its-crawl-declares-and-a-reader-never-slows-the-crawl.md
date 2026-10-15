# A handler pushes Records staged with its attempt into Datasets its crawl declares, and a reader never slows the crawl

A crawl's handler pushes each Record through its context into a Dataset its crawl declares. The Record is staged with the attempt and commits once with it, numbered in its Dataset. A program reads committed Records back as a stream that starts at the Dataset's first Record and never slows the crawl. Sinks are settings, an Export is a file the program saves wherever the engine runs, and every Run keeps a Key-value store.

**What this answers.**
- Pain 11 of the [research on competitors' APIs](https://github.com/goodfetch/goodfetch/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md) (duplicate or partial results on retry), and its open questions 13 (push or pull) and 14 (a schema per Dataset, or plain dicts).
- The [crawl frameworks note](https://github.com/goodfetch/goodfetch/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/crawl-frameworks.md) §4.2 and §6.14.
- What ADRs 0079, 0081, 0082 and 0093 handed over: dropped and rejected Records, where the Records sequence starts, how a Dataset is declared and the code of a rejected Record, and pushing from the handler.

**Competitors, re-read at tagged releases on 2026-10-15.**
- **Crawlee 3.18.2.** The classic crawler writes `context.pushData` at once, so a handler that throws keeps what it pushed ([basic-crawler.ts](https://github.com/apify/crawlee/blob/v3.18.2/packages/basic-crawler/src/internals/basic-crawler.ts#L1668)).
- **Crawlee v4 and Crawlee Python 1.10.4** stage a handler's storage writes until it succeeds. Crawlee v4 still writes queue additions through, and commits "at-least-once" ([upgrade guide](https://github.com/apify/crawlee/blob/8cb2eb61a0356a96205c08e02ed49cadc97081e5/docs/upgrading/upgrading-v4/upgrading-v4.md#L850-L860), [_basic_crawler.py](https://github.com/apify/crawlee-python/blob/v1.10.4/src/crawlee/crawlers/_basic/_basic_crawler.py#L1509)).
- **Scrapy 2.19.0** keeps items yielded before a callback fails, and has no API to iterate items from a script ([scraper.py](https://github.com/scrapy/scrapy/blob/2.19.0/scrapy/core/scraper.py#L399-L438)).
- **Spider 2.52.2's `subscribe()`** loses pages when its reader lags ([website.rs](https://github.com/spider-rs/spider/blob/v2.52.2/spider/src/website.rs#L14296-L14310)).
- **Scrapling 0.4.15's `stream()`** slows the crawl through a buffer of 100 ([engine.py](https://github.com/D4Vinci/Scrapling/blob/v0.4.15/scrapling/spiders/engine.py#L460)).
- **Crawl4AI 0.9.5's `arun_many()`** returns a list or a generator, depending on `config.stream` ([async_webcrawler.py](https://github.com/unclecode/crawl4ai/blob/v0.9.5/crawl4ai/async_webcrawler.py#L1101-L1137)).

Decided in [Crawl output in code](https://github.com/goodfetch/goodfetch/issues/479), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461). Every answer was decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/goodfetch/issues/479#issuecomment-6181693852) and its rounds [6](https://github.com/goodfetch/goodfetch/issues/479#issuecomment-6187750572), [7](https://github.com/goodfetch/goodfetch/issues/479#issuecomment-6188077007), [8](https://github.com/goodfetch/goodfetch/issues/479#issuecomment-6189578875) and [9](https://github.com/goodfetch/goodfetch/issues/479#issuecomment-6189790528), after the fresh reviews and ADR 0093's merge. Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). Datasets and Sinks are Foxhound's (ADR 0084), and so are the Records and Exports made from them. Collie keeps a run's Key-value store, as [Collie's exact capability set](https://github.com/goodfetch/goodfetch/issues/455) decides ([ADR 0088](0088-collie-carries-every-capability-except-those-only-many-pages-need.md)).

## Decision

### Writing: one way to push, one way to read

- **Two tasks, each with one way (S2).**
  - A handler pushes Records into a Dataset in run state, so they checkpoint, resume, reach Sinks under ADR 0008's delivery contract and show in the Console (ADRs 0002, 0003, 0008).
  - A program reads committed Records back as a stream.
  - Crawlee's `pushData` and `getData` split the same way.
- **`ctx.push(dataset, record)`** (`ctx.push(books, Book(...))` in Python, `ctx.push(&books, &book).await?` in Rust).
  - It is on the context, which is the attempt (ADR 0093), as Crawlee's `context.pushData` is (S3).
  - The destination comes first, as in `keyValueStore.set(key, value)`.
  - It is named `push`, not `pushData`. Its destination is required and comes first, and Crawlee 3's `pushData` writes through at once, which is a different meaning (S3).
  - TypeScript types the Record from the Dataset alone (`NoInfer`), so a Record literal missing a field never widens the type.
- **In the SDK API, Records come only from handlers.**
  - There is no `run.push`, so every Record's Provenance names the crawl request that produced it.
  - A program's own state outside a handler goes to the Key-value store.
  - Code-free catalog runs, started from the CLI or as a Schedule's target (ADR 0036), fill Datasets through the engine's own extraction. How they name and declare those Datasets is area 6's and area 11's spec. A Recipe is user code, so these rules apply to it.
- **Not on a handle, nor on the Run.**
  - `books.push(record)` on a handle declared at import would have to find its attempt from hidden context, such as `AsyncLocalStorage` or `contextvars` (G2).
  - A `push` on the Run would exist only on `ctx.run`'s attempt-bound view (ADR 0093), never on the program's own Run.

### Datasets are declared on the crawl

- **The declaration.** `crawl.dataset(name, { schema, key })`, `dataset(name, schema=…, key=…)` in Python, and `crawl.dataset::<T>(name)` or `crawl.dataset_with(name, schema, key)` in Rust.
  - It returns a typed `Dataset<I, O>` handle, with no I/O, as `crawl.label()` does (ADR 0093).
  - With no schema, it takes any JSON object (ADR 0082).
  - The engine knows every Dataset when the Run starts, so a Sink naming one is checked before the first Record (ADR 0026).
- **No default Dataset in the SDK API.** Every push names a declared handle, so a misspelled Dataset is a type error (G3, T). Crawlee's `pushData(data, 'bokks')` silently opens a second Dataset.
- **Mistakes, caught at once:**
  - a second Dataset of one name raises `dataset_duplicate` when it is declared;
  - a push to a handle that another crawl declared raises `dataset_unknown`.
- **Another program reads a Run's Records** with the handle that the crawl's module declares, since declaring does no I/O. The catalog's raw call, by the Dataset's name, is the way down (G8).
- **A resume checks the declarations before any work** (G1, G6), as ADR 0093 checks Labels and ADR 0082 checks Payloads.
  - A Dataset that holds Records but that the crawl no longer declares, a renamed one included, fails the resume with `dataset_unknown`, so no Record is orphaned.
  - A changed key fails it with `dataset_key_invalid`, so a Sink's upsert never mixes two identities.
  - A changed schema is allowed: new Records are checked against it, each Record keeps the hash of the schema it was committed under, and Sinks follow ADR 0026's schema evolution.
  - A Checkpoint holds each Dataset's name, key and schema hash.

### What a push guarantees

- **Staged with the attempt (G1).**
  - `ctx.push` checks the Record and stages it. It resolves once the Record is staged, not once it is committed.
  - The attempt's Records commit together with its enqueues, its seeds added through `ctx.run.addSeed` and its Key-value store writes, under a valid lease and run epoch, once the handler returns (ADR 0003).
  - An attempt whose handler fails, or that ends early (at `skip`, at `retry`, at an escalation, at the handler's timeout, at a lost lease, or when the program closes the Run; ADR 0093), discards them. A retried request therefore commits its Records once, and a failed one leaves none.
  - A `push` after its attempt ended fails with `attempt_ended`, as an `enqueue` does (ADR 0093), so a forgotten `await` never loses a Record silently (G1).
  - **Re-runs.** A handler runs again on a retry, an escalation, a sample comparison and a lost lease (ADR 0093). Only the committing attempt's Records count, in the Dataset and on the outcome; a discarded attempt's model calls still count in Usage (ADR 0093).
  - There is no write-through setting: Crawlee 3's write-through is the behaviour pain 11 complains of.
  - **Writes outside the attempt.** A handler that writes through the program's own Run value, captured from the enclosing code rather than reached through `ctx.run`, makes the program's writes. They commit at once and no lease fences them, so a handler past its lease, or one whose attempt fails, still makes them. They are also made again by every attempt that runs the handler, a discarded one included, such as the cheaper attempt of a sample comparison.
- **Numbering.** A committed Record gets the next number of its Dataset, in commit order, with no gaps (ADR 0008).
- **Sample comparison.** ADR 0020's comparison runs a handler both on the browser and on a cheaper tier. It compares what each attempt staged, before any "Record before commit" Hook: its Records and its enqueues (ADR 0093). A listing page that only enqueues links therefore cannot pass on a tier that finds none. How the comparison weighs the two is area 5's spec.
  - The browser's attempt is the reference, and it is the one that commits, whether the two match or not.
  - Where the comparison also runs with Filter-list blocking off (ADR 0047), the browser attempt whose blocking setting the route keeps is the one that commits.
  - Every other attempt is discarded.
  - If the committing browser attempt fails, the request's attempt has failed, under ADR 0093's retry rules.
- **A Record that fails raises from `push` (G1, G3, G6).**
  - The code is `record_invalid`, naming the Dataset and each issue's path. It covers:
    - a Record that fails its schema, natively in the SDK or against the JSON Schema in the engine (ADR 0082);
    - a value that is not JSON;
    - a Record that lacks its Dataset's key fields.
  - The failing Record is never staged. A handler that catches the error keeps its attempt and its other Records.
  - If the error escapes, the request ends `failed` with `record_invalid` (ADR 0079), which is not retryable, so the request is not retried.
  - Diverting such a Record silently, or a setting that chooses, is rejected: a request would read `succeeded` while its data was missing, and a setting that turns an error into a value is the raise switch ADR 0079 forbids.
- **Dropped and rejected Records (ADR 0033's "Record before commit" Hook stage).**
  - **When the stage runs.** Once the handler returns, before the commit, and only on the Records of the attempt that is about to commit.
    - A discarded attempt's Records never reach a Hook.
    - A Hook that fails (`hook_failed`) fails the attempt, which discards everything it staged and is retried under the work item's policy (ADR 0033).
      > Amended by [ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md): under that policy a Hook's own exception is retried only when `crawl.retry.handler_failed` is on, and a Goodfetch code raised inside it when its retryable flag says so.
  - A Hook's answer never fails the request.
  - A transformed Record is checked against its schema again, and is rejected if it no longer fits.
  - A `succeeded` outcome carries `records`: for each Dataset, how many Records were committed, dropped and rejected. A request that succeeded with nothing committed says why (G1).
  - `run.rejectedRecords(dataset, { after })` reads the Dataset's rejection table, and follows the Run live as `records` does.
    - Each entry carries its `number` in the table (in commit order, with no gaps), so `after` resumes a reader.
    - Each entry also carries the Record as JSON, never parsed by the schema it failed (G3), with the reason, the Hook, the request's ID and its URL.
  - Dropped Records are counted by reason in the run's report, never stored.
  - A Sink's dead letters stay ADR 0026's.
- **Duplicates.**
  - **Retries:** staging prevents them.
  - **Deliveries:** the Record's Dataset and number, with ADR 0008's delivery ID and run epoch, keep a repeated delivery from adding a Record, as far as the Sink's Guarantee class allows (ADR 0026).
    - A Record's number is unique only within its Dataset, so a delivery identifies a Record by both.
    - ADR 0026's record ranges and its order by record number are per Dataset.
  - **The same thing collected twice** (a book in two categories, or in tomorrow's run): the Dataset's declared `key`.
    - The key lists top-level fields that the schema requires. In TypeScript they are typed as the schema's field names, and a field the schema does not require raises `dataset_key_invalid` when the Dataset is declared.
    - The Dataset stays append-only.
    - Dropping a duplicate before commit is a Hook's job, with its reason counted.
    - **Keyed write modes** (upsert, insert-only, replace, history; ADR 0026) use the Sink's own `key`, or else each Dataset's own key. The Run's start refuses, with `setting_invalid` naming `sinks.<name>.key`:
      - a Sink in a keyed mode with no key of its own when any Dataset it receives has no key;
      - a Sink's own key naming a field that a schema of one of its Datasets does not require.
    - A Record that reaches a keyed Sink without its key fields, from a Dataset with no schema, joins the Sink's dead letters with that reason (ADR 0026).
    - **Change tracking against a Baseline** identifies a Record across runs by its Dataset's key (ADRs 0026, 0036). How it compares the Records of a Dataset with no key is area 6's spec.

### Reading Records

- **The call.** `run.records(dataset, { after })` on a crawl's Run: `records(books, after=…)` in Python, and `records(&books)` or `records_after(&books, n)` in Rust. `rejected_records` and `rejected_records_after` follow the same form.
  - It is an iterator, subscribed when called (ADR 0081).
  - It yields `DatasetRecord<O>`: `number`, `value` (the schema's output type, built by its native parse, ADR 0082), `requestId`, `url` and `committedAt`. `Record` alone would shadow TypeScript's built-in `Record` type.
- **Where it starts.** At the Dataset's first Record, or after `after`, which resumes a reader from the last number it handled (ADR 0008). Starting at the moment of the call would silently miss the Records committed before it (G1); Spider's `subscribe()` does.
- **When it ends.**
  - It follows the Run live, and ends once the Run has ended and every Record has been read.
  - On a Run that has already ended, it reads to the last Record.
  - Through a handler's `ctx.run`, `records` and `rejectedRecords` read what has committed when called, then end: the Run cannot end while the handler holds its request. `export` works there as it does on the Run.
  - Through the program's own Run value, captured by a handler, they follow the Run live as anywhere else and wait until the handler's timeout. A handler reads through `ctx.run`.
- **A failed parse.** A Record that its native parse rejects ends the iteration with `record_invalid`, carrying its number, so the reader can resume after it.
- **Only a crawl's Run has Records** (ADR 0084), so `records` is on `CrawlRun`, not on every `Run`.
- **A reader never slows the crawl (G1).**
  - Records are committed to run state before anyone reads them, so a reader is a cursor over them. The engine sends what the reader's credit allows, and the crawl never waits for it.
  - Records are lossless because they are durable. Spider's channel loses pages and Scrapling's stream slows the crawl; a paused debugger, a slow call per Record or a forgotten iterator would otherwise hold every lease.
  - Handler requests, which are the crawl's work, still slow their producer (ADR 0024).

### Exports

- **The call.** `run.export(dataset, { format, compression })` writes an Export of the Records committed so far (ADR 0026).
- **What it returns.** An `Export` with its format, its Record count, its last Record number, its size and its SHA-256.
  - `saveAs(path)` is Playwright's `Download` name for a file the engine holds (S3).
  - `stream()` returns an async iterable of bytes (an `EventStream` in Rust), not the Node `Readable` of Playwright's `createReadStream()`, so it has a name of its own (S3).
- **Wherever the engine runs.** A path on the engine's own machine would break on a Daemon or in Goodfetch Cloud (ADR 0001; the research's "works locally but not on the Apify Platform").
- **The format is named, never inferred** from a file name, as Crawlee's `exportData` infers it, so a typo is a type error (G3). Each format's options are `export.*` settings, named by area 6.

### The Key-value store

- **On every Run, Collie's included (ADR 0088):** `run.keyValueStore` (`run.key_value_store` in Python, `run.key_value_store()` in Rust).
- **JSON values.**
  - `get(key)` returns JSON, or null when the key is absent: a question's answer (ADR 0079).
  - `get(key, schema)` returns the schema's output type, built by its native parse.
  - `set(key, value)` takes JSON, or in Python a Pydantic model.
  - `set(key, null)` is refused with `value_invalid`, naming `delete` in its `fix`. A null from `get` therefore always means an absent key, in every language.
  - In TypeScript, `set`'s type excludes a top-level null (G3), so only Python, Rust and an untyped caller meet the run-time refusal. A nested null is kept.
- **Bytes.** `getBytes(key)` returns `{ content, contentType }` or null, and `setBytes(key, content, contentType)` writes them.
- **Removing and listing.** `delete(key)` returns whether the key existed. `keys({ prefix })` is an iterator.
- **One code for a wrong value.** These raise `value_invalid`, naming the key:
  - a value that is not JSON, or null;
  - a value that fails its schema;
  - bytes read with `get`, whose `fix` names `getBytes`.
- **Map-like names (S1).** Crawlee's `getValue` returns JSON or a `Buffer` depending on the content type, one method with two return types (S2), so its names are not taken (S3 applies only where the meaning matches).
- **In a handler, the store is reached through `ctx.run`.** ADR 0093 binds `ctx.run` to its attempt, so its writes stage with the attempt and its reads see the attempt's own writes first, as `ctx.run.addSeed` stages.
  - A cursor, or a blob that a Record points to, then commits with the work it belongs to.
  - The program's own Run value writes at once, for state that must outlive a failed attempt, outside the attempt's fence (above).
- **Concurrency.**
  - For each key, the last commit wins.
  - A read sees the committed values and its own attempt's staged writes; another attempt's staged writes stay invisible until it commits.
  - There is no compare-and-set. Two attempts that `get` and then `set` one key keep the later commit's value.
  - A value that concurrent handlers share live stays in the program's own memory, within one process. Across worker processes (ADR 0024's `workers=N`) there is no such value, and a program counts from committed Records or outcomes instead.
- **No `useState`.**
  - Crawlee's `useState` hands every handler one live object, saved on a 60 s timer and at close, outside any commit; Crawlee v4 still leaves it out of its transactional writes. A failed attempt's changes to it would survive (pain 11, applied to state).
  - One live object cannot span the program's worker processes (ADR 0024's `workers=N`) or a resume on another machine.
  - `get(key, schema)` and `set` keep what must survive a restart.

### Sinks in code

- **Sinks are settings.** `sinks.<name>.*` is one family per Sink: its type, the Datasets it receives, its destination (a Secret reference), its write mode and its key, defaulting to its Datasets' keys.
  - It is set on a connection or a crawl's Run, from code, the project file, the CLI or a Recipe alike (ADRs 0028, 0083), because where data goes differs by deployment while the crawl's logic does not.
  - It is Run-wide, so a Label refuses it with `setting_not_applicable` (ADR 0093).
  - **On a Collie Run,** which has no Datasets, only a value set where it applies is refused, so one project file serves a project's Collie programs and its crawls (ADR 0083):
    - On a Foxhound engine, a value set on the Collie Run itself is refused with `setting_not_applicable`. A Sink inherited from the connection or from configuration is inactive for the Collie Run, as `explainSettings()` shows.
    - A Collie engine refuses a value given to a Run or a connection with `not_in_product` (ADRs 0087, 0088). User and project configuration and Operator policy may hold `sinks.*`: a Collie engine applies none of it, and `config explain` lists it as Foxhound's (ADR 0087).
  - Each Sink type's keys are area 6's spec.
- **Checked at start (G3).**
  - A Sink set on the Run itself that names a Dataset the crawl does not declare is refused with `setting_invalid`, naming the closest Dataset.
  - A Sink inherited from the connection or from configuration delivers only the Datasets it names that the crawl declares. It is inactive for a crawl that declares none of them, as `explainSettings()` shows, so one project file can serve several crawls (ADR 0083).
  - **A departure from G3, named.** A misspelled Dataset in an inherited Sink is never refused, since that Sink may serve another crawl. `explainSettings()` and the run's report show it inactive.
- **A Sink written in the program** (ADR 0026's SDK-handler form) is a Plugin in code. [Hooks and injectable components in the SDK API](https://github.com/goodfetch/goodfetch/issues/481) decides it.
  > Amended by [ADR 0104](0104-a-crawl-declares-its-hooks-per-stage-in-code-a-hook-returns-its-change-and-a-program-registers-its-own-plugins-by-name.md): it is registered on `connect()` by name (`sinks`), serves the `sinks.<name>` entry of its name, and takes no `type` there.
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

- **Amends ADR 0002.** A Checkpoint holds each Dataset's name, key and schema hash. A note sits under its opening.
- **Amends ADR 0003.** Writes a handler makes through the program's own Run value are outside the attempt and its lease. A note sits under its opening.
- **Amends ADR 0020.** A sample comparison compares the Records and enqueues each attempt staged; the browser attempt whose blocking setting the route keeps commits, and every other attempt is discarded. A note sits under its opening.
- **Amends ADR 0036.** Change tracking identifies a Record across runs by its Dataset's key. A note sits under its "Acting on change" bullet.
- **Amends ADR 0008.** Each Dataset numbers its own Records, from 1; a reader reads one Dataset's stream, and a delivery identifies a Record by its Dataset and number. A note sits under its results-stream Consequence.
- **Amends ADR 0024.** Records read through a crawl's Run come from run state as a cursor and never slow the producer; handler requests still do. A note sits under its event-delivery bullet.
- **Amends ADR 0081.** An unread Records sequence does not slow its Run, and `records` is on a crawl's Run. A note sits under its Iterators bullet.
- **Amends ADR 0079.** A `succeeded` crawl request outcome counts its Records per Dataset: committed, dropped and rejected. A note sits under its outcomes bullet.
- **Amends ADR 0033.** The "Record before commit" stage runs only on the committing attempt's Records; a transformed Record is checked against its schema again; and a program reads the rejection table with `run.rejectedRecords()`. A note sits under its Hook stage table.
- **Amends ADR 0026.** Keyed write modes use the Sink's own key, or else each Dataset's key, checked at start; record ranges and their order are per Dataset; and Sinks are configured as `sinks.<name>.*` settings. A note sits under its write modes.
- **Amends ADR 0093.** Records are pushed with `ctx.push`, which joins the context's members, not through `ctx.run`. Notes sit under the context's `run` member and under its hand-off to this ticket.
- **ADR 0037's matrix.**
  - CO-25, CO-26, CO-27, CO-29, RT-09 and pain 23 cite this ADR; RT-09's `useState` is declined.
  - Areas 3, 5, 6, 10 and 11 list it.
- **`GLOSSARY.md`.** Dataset and Record are sharpened: a Dataset is named by the crawl that fills it and may be keyed by the fields that identify a Record, and a Record is committed once with the work that produced it.
- **Who builds what.**
  - **Area 3 (Engine runtime):** staging Records and Key-value store writes with the attempt, the "Record before commit" stage at commit with its check of a transformed Record, per-Dataset numbering, the reader's cursor, the resume checks, and the Checkpoint's Dataset entries.
  - **Area 5 (Site access):** comparing the Records and enqueues each attempt staged in a sample comparison, and committing the browser attempt whose blocking setting the route keeps.
  - **Area 6 (Collection):** Datasets with keys and their checks, the rejection table and its counts, Exports in code, the Key-value store's operations, the `sinks.*` and `export.*` keys, WARC's keys, change tracking without a key, and how code-free crawls name their Datasets.
  - **Area 10 (Scheduling):** change tracking by key for a Schedule's Baseline.
  - **Area 11 (Client surfaces):** `crawl.dataset`, `ctx.push`, `records`, `rejectedRecords`, `export`, the Key-value store, and each language's error classes.
- **Other tickets of the map.**
  - [The Run in code](https://github.com/goodfetch/goodfetch/issues/478) reads outcomes with their Record counts, each Sink's delivery status, and opens a past Run by ID.
  - [Hooks and injectable components in the SDK API](https://github.com/goodfetch/goodfetch/issues/481) shapes the "Record before commit" Hook and SDK-handler Sinks.
  - [Testing a user's crawl or automation](https://github.com/goodfetch/goodfetch/issues/482) records what a test handler pushed.
    > Amended by [ADR 0105](0105-a-programs-tests-replay-a-response-recording-in-a-test-engine-that-reaches-no-site.md): `runHandler`'s `AttemptResult.records(dataset)` returns each Record the attempt pushed, as its Dataset's schema parses it, even when a crawl would discard the attempt; a Record's committed form and what Hooks drop or reject are read with `run.records()` and `run.rejectedRecords()` from a crawl started on a Response recording.
- **The `prototype/sdk-api` sketch** holds these signatures in its `output` files for each language, with their snippets.
