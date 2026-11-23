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
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console. The engine behaviour this ADR describes stands and the Console's part does not; a Client reaches it.
  - A program reads committed Records back as a stream.
  - Crawlee's `pushData` and `getData` split the same way.
- **`ctx.push(dataset, record)`** (`ctx.push(books, Book(...))` in Python, `ctx.push(&books, &book).await?` in Rust).
  - It is on the context, which is the attempt (ADR 0093), as Crawlee's `context.pushData` is (S3).
  - The destination comes first, as in `keyValueStore.set(key, value)`.
  - It is named `push`, not `pushData`. Its destination is required and comes first, and Crawlee 3's `pushData` writes through at once, which is a different meaning (S3).
  - TypeScript types the Record from the Dataset alone (`NoInfer`), so a Record literal missing a field never widens the type.
  > Note (agent decision, as built; [#956](https://github.com/goodfetch/goodfetch/issues/956)): on the wire, `crawl.start` takes `datasets`, a list of `{ name, schema?, key? }`, so the engine itself refuses a second Dataset of one name with `dataset_duplicate` and a key on a field that is no top-level field the schema's `required` lists with `dataset_key_invalid`, before the Run begins. `ctx.push` is `crawl_request.push { run_id, crawl_request_id, lease_epoch, dataset, data }`, answered `{}` once staged: the engine checks the Record against the run file's declarations (`dataset_unknown`, naming the declared Datasets; `record_invalid`, naming each issue by its JSON Pointer path, for a Record that is no JSON object, fails its schema, or lacks a key field of a Dataset with no schema) and stages it with the attempt, refusing it with `attempt_ended` once the attempt ended. `crawl_request.handle`'s answer keeps its `records`, which commit after the pushed ones; one that does not hold fails the request with `record_invalid`, not retried, and nothing the attempt staged commits, as an escaped `record_invalid` would. A sample comparison compares each run's pushed Records, then its answer's. Declarations live in the run file's `datasets` table (name, position, schema, schema hash, key) beside `records`, both in `goodfetch-dataset`.
  > Amended by [ADR 0147](0147-a-pages-fields-are-read-in-one-call-on-every-tier-and-a-push-takes-one-record-or-several.md) (owner's decisions of 2026-11-10, grilling of G-01 of the consolidated review): `ctx.push(dataset, recordOrList)` also takes a list, sent as one message; on the wire `crawl_request.push` carries the Records of one call and answers one outcome per Record. `push` stays one checked round trip: no SDK holds a Record back for the handler's answer.
- **In the SDK API, Records come only from handlers.**
  - There is no `run.push`, so every Record's Provenance names the crawl request that produced it.
  - A program's own state outside a handler goes to the Key-value store.
  - Code-free catalog runs, started from the CLI or as a Schedule's target (ADR 0036), fill Datasets through the engine's own extraction. How they name and declare those Datasets is area 6's and area 11's spec. A Recipe is user code, so these rules apply to it.
    > Note (agent decision, on a default, not read by the owner), 2026-10-26, [#994](https://github.com/goodfetch/goodfetch/issues/994): on the wire, `crawl.start` takes `extraction { formats, follow? }` (`follow`: `scope`, the default, `sitemap` or `none`; `fields` are #1002's). The engine declares the Label `page` beside any the start declares and the Dataset `pages`, keyed by `url`, whose schema is `page_formats_schema(formats)` with `status` (integer or null) and `fetchedAt` (RFC 3339, UTC) required beside it; a start that also declares a Dataset fails with `invalid_params`. Each attempt reads its page through `page.formats` (served or browser page alike) and answers one Record: the formats as `page.formats` answers them, so `url` is the document's after redirects, plus the response's `status` and `fetchedAt`; it commits under the lease as a handler's answer does, and a page of any status the crawl hands over (a 404 among them) gets its Record. `scope` admits every `a[href]` of each page as `enqueueLinks` does, staged with the attempt; `sitemap` adds, beside the seeds, one robots.txt sitemap seed per request seed's origin with the rule `**` → `page`, and follows no page's links; `none` fetches the seeds alone. With no `fetch.tier` set the engine chooses each request's tier, as under `auto`, since every request must be fetched. The extraction is kept in the run file's `code_free_extraction` table, so a resumed crawl handles its requests alike.
    > Note (agent decision, as built), 2026-10-28, [#994](https://github.com/goodfetch/goodfetch/issues/994) (step 7 review, finding 580-582-7): a code-free Record spells every key as the protocol does, in snake_case: `fetched_at` beside `page.formats`' own `base_url`, `main_content`, `open_graph` and `json_ld`, in the Record and in the schema of `pages`. The spec's `fetchedAt`, `baseUrl` and `mainContent` are its TypeScript spellings of those names, as it writes `run.addSeed` for `run.add_seed`; every SDK's generated types keep the wire's snake_case, so a code-free Record reads like a Record a handler pushes from `page.formats`' answer.

    > Note (agent decision, accepted by the owner on 2026-10-28 as amended by owner decision O23 in the note of 2026-10-30 below), 2026-10-26, [#1002](https://github.com/goodfetch/goodfetch/issues/1002): `extraction.fields` maps each field's name to `{ locator: { kind: css | xpath, selector }, attribute?, match?: { pattern, group = 1 }, transform?: { kind: text | number | date | date_time, locale, time_zone? } }` (`text` by default). A field's name may not be a property every Record holds (a format's, `status`, `fetched_at`), and a CSS selector the engine does not read, a pattern that does not compile or lacks its group, a locale that is no BCP 47 tag or an unknown time zone fail the start with `invalid_params`. Each field reads the first element its locator finds: its text, white space collapsed, or the attribute named, an `href` or `src` resolved against the base URL; a served page reads CSS selectors only, so an `XPath` field escalates it to the browser under `auto` (Provenance's `call` is `crawl.extraction.fields`) and fails `browser_required` under a forced cheaper tier. The schema of `pages` requires each field, typed by its transform or null. A field the page does not give is null, and the Record's Provenance lists it in `unread_fields` with its reason (`no_element`, `no_attribute`, `no_match`, `not_parsed`, `no_time_zone`); `value_transforms` names each transform with its text, locale and zone. The CLI's `--field name=selector` (area 11) maps to `{ locator: { kind: css, selector } }`.

    > Note (owner decision O23 (a), 2026-10-28; its reading below an agent decision, on a guarantee, not read by the owner), 2026-10-30, deep review item 582-O3 ([#1748](https://github.com/goodfetch/goodfetch/issues/1748)): a served page reads an `XPath` field too, so no field escalates a page and none fails `browser_required`; the spec's table serves `XPath` locators on document tiers, and a code-free crawl with one under `fetch.tier: http` fills it from the served document. The engine evaluates `XPath` 1.0 over the document as parsed for CSS selectors, as a browser's `document.evaluate` reads an HTML document: a name test without a prefix finds an HTML element, or an HTML element's attribute, in any letter case and no element of another namespace; `xmlns` attributes and namespace nodes are on no axis; a `<template>`'s contents are not in the document; the doctype is a node only `node()` finds; `lang()` reads only an `xml:lang` in the XML namespace; strings are measured in characters. The field reads the first node in document order the expression finds, and an expression whose first node is no element finds none (`no_element`). The start refuses with `invalid_params` an expression a browser refuses with no namespace resolver and no variables (a syntax error, a namespace prefix, a variable, a function outside the core library or a wrong number of arguments), one that answers a value rather than nodes (`count(//a)`), and one nested more than 100 levels deep. The engine's evaluator is its own: no maintained crate evaluates `XPath` 1.0 over an HTML tree with a browser's name matching.
- **Not on a handle, nor on the Run.**
  - `books.push(record)` on a handle declared at import would have to find its attempt from hidden context, such as `AsyncLocalStorage` or `contextvars` (G2).
  - A `push` on the Run would exist only on `ctx.run`'s attempt-bound view (ADR 0093), never on the program's own Run.

### Datasets are declared on the crawl

- **The declaration.** `crawl.dataset(name, { schema, key })`, `dataset(name, schema=…, key=…)` in Python, and `crawl.dataset::<T>(name)` or `crawl.dataset_with(name, schema, key)` in Rust.
  > Amended by [ADR 0099](0099-every-surface-spells-each-catalog-name-by-its-own-rule-shapes-come-from-declared-kinds-and-call-is-the-way-down.md): Rust's is one `crawl.dataset`, taking `impl Into<DatasetDeclaration<T>>`: a bare name, `crawl.dataset::<T>(name)`, or `crawl.dataset(DatasetDeclaration::<T>::new(name).schema(schema).key(&[…]))`, whose builder methods are named after TypeScript's `schema` and `key` options; there is no `dataset_with`.
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

  > Note (agent decision, accepted by the owner on 2026-10-28; [#970](https://github.com/goodfetch/goodfetch/issues/970)): on the wire, the checks run at `crawl.start` under the ID of an existing Run, beside ADR 0082's Payload check, against the run file's `datasets` table: a Dataset that holds at least one committed or rejected Record and is no longer declared fails with `dataset_unknown` (`declared` naming the Datasets the start declares); one whose key differs, order included, with `dataset_key_invalid`, whose details gain `reason`: `not_required` with `field` at declaration, `changed` with `key` and `began_with` at a resume. A rejected Record counts as a Record (owner, 2026-10-28, O24), so a Dataset whose every Record failed its schema keeps its rejection table readable under its name through `run.rejected_records`, as #964 promises. A Dataset that holds no Record, committed or rejected, may be dropped or keyed anew. A start under the ID of a Run that no crawl of this engine works keeps its declarations, a code-free crawl's `extraction` among them, until that Run resumes here (`run.resume`), when they replace those in the run file in one write before any work, a Dataset no longer declared that holds no Record dropped; a start under the ID of a crawl this engine works changes none of its declarations, as it changes none of its Labels. Each row of `records` keeps `schema_hash`, the hash of the schema its Dataset was declared with when it committed (none without a schema).

### What a push guarantees

- **Staged with the attempt (G1).**
  > Amended by [ADR 0106](0106-a-program-declares-an-effect-as-a-keyed-block-whose-output-the-effect-journal-keeps-judged-by-what-the-engine-sent.md): while a declared Effect's body runs in an attempt, a `ctx.push`, an enqueue, a seed or a Key-value store write staged with that attempt fails with `effect_staging_refused`, since a re-run that finds the Effect in the Effect journal does not run the body again.
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
  > Amended by [ADR 0147](0147-a-pages-fields-are-read-in-one-call-on-every-tier-and-a-push-takes-one-record-or-several.md) (owner's decisions of 2026-11-10, grilling of G-01 of the consolidated review): for a list, each Record is checked, the valid ones are staged, and the `record_invalid` raised at that `push` names each failing Record by its position and each issue by its path.
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
    > Note (agent decision, on a guarantee, not read by the owner; [#964](https://github.com/goodfetch/goodfetch/issues/964)): until Hooks exist, the engine's own check fills the table. A Record that `crawl_request.push` refuses with `record_invalid` still raises, and is never staged as a Record; its rejection is staged with the attempt in its place, so it enters its Dataset's rejection table, and counts as `rejected` on the outcome, only when that attempt commits (a discarded attempt's never does), and a request whose every Record was rejected says `none_kept`. A push into an undeclared Dataset (`dataset_unknown`) enters no table, and an answer's Record that does not hold still fails the request. On the wire the reader is `run.rejected_records { run_id, dataset, after? }`, answering pages of `{ records, next, ended }` as `run.outcomes` does, each entry `{ number, data, reason, issues, hook?, request_id, url }`, and refusing an undeclared Dataset with `dataset_unknown`. The table is the run file's `rejected_records` (dataset, number, work item, the Record, and why as JSON), in `goodfetch-dataset`.
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
    > Amended by [ADR 0102](0102-a-program-names-its-run-by-an-id-that-a-start-resumes-ends-it-with-a-run-report-and-reads-what-waits-and-why.md): on a Run with no live writer, which has not ended, a view's `records()` and `rejectedRecords()` end after the last committed Record, as its `progress()` and `outcomes()` do, rather than wait for a program to resume it.
  - On a Run that has already ended, it reads to the last Record.
  - Through a handler's `ctx.run`, `records` and `rejectedRecords` read what has committed when called, then end: the Run cannot end while the handler holds its request. `export` works there as it does on the Run.
  - Through the program's own Run value, captured by a handler, they follow the Run live as anywhere else and wait until the handler's timeout. A handler reads through `ctx.run`.
- **A failed parse.** A Record that its native parse rejects ends the iteration with `record_invalid`, carrying its number, so the reader can resume after it.
- **Only a crawl's Run has Records** (ADR 0084), so `records` is on `CrawlRun`, not on every `Run`.
- **A reader never slows the crawl (G1).**
  - Records are committed to run state before anyone reads them, so a reader is a cursor over them. The engine sends what the reader's credit allows, and the crawl never waits for it.
  - Records are lossless because they are durable. Spider's channel loses pages and Scrapling's stream slows the crawl; a paused debugger, a slow call per Record or a forgotten iterator would otherwise hold every lease.
    > Note (agent decision, on a guarantee, not read by the owner), 2026-11-06, [#1577](https://github.com/goodfetch/goodfetch/issues/1577) ([ADR 0138](0138-a-page-is-analysed-once-costly-features-are-settings-and-run-state-commits-in-groups-at-a-chosen-durability-level.md)): a Record is as durable as the Run's Durability level. At `full`, the default, its attempt's outcome is synced before a reader can read it. Below `full`, a reader can read a Record that a power cut then takes back: at `fast` any recent commit, at `periodic` at most the last `run_state.commit_interval` of commits ([#1927](https://github.com/goodfetch/goodfetch/issues/1927)), and at `off` a crash discards the run file with every Record, and the Run starts again. The run report's `weakened` names the level, so a reader knows what it read may not survive. A Record commits in its attempt's one outcome job, with the attempt's enqueues and the Lease's commit ([#1925](https://github.com/goodfetch/goodfetch/issues/1925)). Readers read on the Run's kept read-only connections beside the writer, so a read never queues behind the crawl's writes ([#1923](https://github.com/goodfetch/goodfetch/issues/1923)). A crawl's Datasets and their validators are built once, when the crawl starts or resumes, and every push is checked against them ([#1906](https://github.com/goodfetch/goodfetch/issues/1906)). `dataset.records: false` keeps no Record for a crawl that only fetches and follows: `run.records` reads none, each succeeded request's outcome says `records_off`, and a Record named for a Dataset the crawl does not declare is still refused with `dataset_unknown` ([#1921](https://github.com/goodfetch/goodfetch/issues/1921)).
  - Handler requests, which are the crawl's work, still slow their producer (ADR 0024).
    > Note (agent decision, as built; [#961](https://github.com/goodfetch/goodfetch/issues/961)): on the wire, `run.records { run_id, dataset, after? }` is a sequence. The engine sends each Record in a `$/record { id, record }` notification, one per credit the Client grants with `$/credit`; the Record carries `number`, `value` (its JSON as committed, from which the SDK's native parse builds `DatasetRecord<O>`), `request_id`, `url` (its crawl request's) and `committed_at`. The call answers `{ count, last }` once the Run has ended, or has no live writer, and every Record is sent; a Dataset the crawl does not declare fails with `dataset_unknown`. The `records` table keeps each Record's request URL and commit time beside its request's ID, so a reader, an Export and a Sink read one table through one query, where a filter such as the Wall marker's (ADR 0052) joins. The engine reads a page of Records on the Run reader's read-only connection, then waits for credits holding no read transaction, so a reader that grants none holds back neither the writer nor its WAL checkpoints. #319's budget counts instructions and heap per benchmark, and no test times a crawl (ADR 0071), so "never slows the crawl" is proved by a crawl that sends every request, commits every Record and ends while its reader holds no credit.
    > Note (agent decision, on a default, not read by the owner), 2026-10-28, [#961](https://github.com/goodfetch/goodfetch/issues/961) (step 7 review, finding 581a-4): a reader on an engine that does not work the Run, such as a reader on the Daemon of a Run an Attached engine crawls, follows it live too: while another engine holds the Run's writer lock, which the reader tests without taking it, the reader reads the run file again every second (the engine's clock), so `run.records`, and `run.outcomes` and `run.rejected_records` when the owner did not answer them, end only once no engine holds the lock. The Run's own engine is still followed by its commits, never polled.

### Exports

- **The call.** `run.export(dataset, { format, compression })` writes an Export of the Records committed so far (ADR 0026).
- **What it returns.** An `Export` with its format, its Record count, its last Record number, its size and its SHA-256.
  - `saveAs(path)` is Playwright's `Download` name for a file the engine holds (S3).
  - `stream()` returns an async iterable of bytes (an `EventStream` in Rust), not the Node `Readable` of Playwright's `createReadStream()`, so it has a name of its own (S3).
- **Wherever the engine runs.** A path on the engine's own machine would break on a Daemon or in Goodfetch Cloud (ADR 0001; the research's "works locally but not on the Apify Platform").
  > Amended by [ADR 0116](0116-a-saved-file-is-written-whole-or-not-at-all-and-a-failed-write-says-whether-its-path-or-its-storage-failed.md): `run.export` returns an `Export` only once its file is complete, and fails with `file_write_failed` (an `UnavailableError`, side `engine`) when the engine cannot write it. `saveAs` writes a hidden temporary file beside the path and renames it over the path, so the path holds the whole Export or what it held before. It creates missing parent folders and replaces an existing file, as Playwright's does, and fails with `path_not_writable` (a `ProgramError`) when the program's path cannot be written, or `file_write_failed` (side `program`) when its storage is full or fails. Neither code is retryable.
- **The format is named, never inferred** from a file name, as Crawlee's `exportData` infers it, so a typo is a type error (G3). Each format's options are `export.*` settings, named by area 6.
  > Amended by [ADR 0119](0119-every-fetch-tier-decodes-a-crawls-document-as-the-browser-does-a-label-names-the-content-types-its-handler-takes-and-exports-are-utf-8.md): every text format is UTF-8, and CSV's BOM is the setting `export.csv.bom`, off by default.
  > Note (agent decision, on a default, not read by the owner), 2026-10-26, [#980](https://github.com/goodfetch/goodfetch/issues/980): on the wire, `run.export { run_id, dataset, format, compression }` answers `{ export_id, format, compression, records, last_number, size, sha256 }` once the file is whole. `export_id` is `<run_id>.export-<sha256>`, which `export.save_as { export_id, path? }` takes (as `download.save_as`: an Attached engine writes the path, any other sends `$/chunk`s) and the sequence `export.stream { export_id }` (`$/chunk`s, one per credit, answering `{ size, sha256 }`). An Export holds the Records numbered up to the last one committed when the call starts, read a page at a time on the Run reader's connection, so it serves a live Run and one that ended; each value is its JSON text as committed. `json` is a compact array with no trailing line feed, `jsonl` one value per line, each ended by a line feed. The file is kept under the Run folder's `exports/`, named by its SHA-256 and recorded in no run state, since an ended Run has no writer: it goes with the Run, and no Checkpoint carries it. `compression` has only `none` until #989 adds gzip and zstd.
  > Note (agent decision, on a default, not read by the owner), 2026-11-23, [#2348](https://github.com/goodfetch/goodfetch/issues/2348): an Export carries the run epoch of the Run state it was written from, as every write leaving the engine does (ADR 0003): `run.export`, `export.save_as` and `export.stream` answer `run_epoch`, `export_id` is `<run_id>.export-<run_epoch>-<sha256>`, and the file is kept under `exports/<run_epoch>/`, so a destination that compares and sets the run epoch keeps only the current copy's Exports.
  > Note (agent decision, on a default, not read by the owner), 2026-10-26, [#984](https://github.com/goodfetch/goodfetch/issues/984): `csv`, `xlsx`, `html_table` and `markdown` read the Records twice: first for the columns, the union of every field seen in first-seen order (each Record's fields in its committed text's order), then for the rows, so a later Record's field is never dropped (pitfall D07). A cell holds a string's text; a number, `true` or `false` as JSON writes it; nothing for `null` or a missing field; and an object or array as its JSON text, or, in CSV under `export.csv.nested: flatten`, an object's fields in dotted columns (an array stays JSON text). CSV follows RFC 4180 with CR LF line ends. XLSX is a streamed ZIP of one sheet of inline strings, its entries dated 1980-01-01 so equal Records give equal bytes. The HTML table is a whole UTF-8 document titled after the Dataset. Markdown is a GitHub Flavored Markdown table, with each line break written `<br>`. `run.export` takes `settings` and `project`: the run layer and project configuration its `export.*` settings resolve under, with the machine's layers, so the SDK passes the connection's and the Run's values there. An `export.csv.delimiter` that is not one character other than `"`, CR or LF, and an `export.xlsx.sheet` Excel would not take, fail with `setting_invalid`, each checked only for its own format.
  > Note (agent decision, on a default, not read by the owner), 2026-10-26, [#989](https://github.com/goodfetch/goodfetch/issues/989): the other formats are `xml`, `parquet`, `avro`, `arrow_ipc`, `rss` and `atom`, and `compression` is `none`, `gzip` or `zstd` over the whole file. Parquet, Avro and Arrow IPC read the Records twice, as the tables do, and type each nullable column from every value seen in it: boolean, 64-bit integer, 64-bit float, or a string holding each cell's text (an object or array as its JSON text). Avro is written by hand (an object container file, `null` codec, its sync marker from the schema's SHA-256) so equal Records give equal bytes; a field name Avro cannot take is made one, with the field's own name in its `doc`. Parquet's codecs are `zstd` (the built-in), `snappy` and `none`: its gzip codec would turn on the zlib-rs deflate backend for the whole build. XML writes each field as an element (`field` with a `name` attribute where its name is no XML name), an array's values as `item`s and `null` as `xsi:nil`; a character XML 1.0 cannot hold becomes U+FFFD. RSS and Atom take each item's title, link, date and summary from the fields `export.feed.*` name; Atom's feed `updated` is its latest entry's, written after the entries. `export.json.indent` takes 0 to 8. A Record read past a Wall (ADR 0052) carries its Provenance's marker as a `#wall` field in every format but the feeds, where it is a category of the scheme `urn:goodfetch:wall`; it replaces a Record's own `#wall` field. `run.records` and `run.export` take `wall: true | false` to read only the Records with, or without, one.

### The Key-value store

- **On every Run, Collie's included (ADR 0088):** `run.keyValueStore` (`run.key_value_store` in Python, `run.key_value_store()` in Rust).
- **JSON values.**
  - `get(key)` returns JSON, or null when the key is absent: a question's answer (ADR 0079).
  - `get(key, schema)` returns the schema's output type, built by its native parse.
  - `set(key, value)` takes JSON, or in Python a Pydantic model.
  - `set(key, null)` is refused with `value_invalid`, naming `delete` in its `fix`. A null from `get` therefore always means an absent key, in every language.
  - In TypeScript, `set`'s type excludes a top-level null (G3), so only Python, Rust and an untyped caller meet the run-time refusal. A nested null is kept.
- **Bytes.** `getBytes(key)` returns `{ content, contentType }` or null, and `setBytes(key, content, contentType)` writes them.
  > Note (agent decision, as built; [#792](https://github.com/goodfetch/goodfetch/issues/792)): on the wire, `key_value_store.get_bytes` answers `{ bytes }`, null for an absent key, else `{ content, content_type }` with `content` in Base64 (RFC 4648, with padding), and `set_bytes` takes `content` and `content_type` and answers `{}`; content that is no Base64 is `invalid_params`, and the content type is kept as given. `value_invalid`'s reasons gain `value_bytes` (`get` on bytes, its `fix` naming `getBytes`) and `value_json` (`getBytes` on JSON, its `fix` naming `get`). A row of `key_values` holds JSON in `value` or bytes with their `content_type`: up to 1 MiB in `content`, larger ones as a content-addressed file of the Run's folder recorded in `run_files` (kind `key_value`, named by the key) and referenced by its position in `file`, which Checkpoints carry as they carry a Download's file; deleting or replacing the key deletes the file once no record of the Run names it.
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

  > Note (agent decision, accepted by the owner on 2026-10-22; [#791](https://github.com/goodfetch/goodfetch/issues/791)): on the wire, `key_value_store.get` answers `{ value }`, null for an absent key; `set` answers `{}` once committed; `delete` answers `{ existed }`; and `keys` is a sequence that sends each key in a `$/key` notification (`{ id, key }`), one per credit, as `page.body` sends chunks, then answers `{ count }`. Every Run's file holds the store's table `key_values` (`key`, `value` as JSON text), which the engine's own contribution adds, so it travels in Checkpoints. Reads go through the Run reader's read-only connection, so a view reads a Run no program holds without its writer lock; `set` and `delete` go through a live Run's writer, and on a Run with no live writer here answer `run_not_found`, naming a start or resume. `value_invalid`'s details carry `key`, `refused` (`key` or `value`), `reason` (`key_empty`, `key_too_long`, `key_control_character`, `value_null`) and `fix`; a control character is a Unicode `Cc` scalar value.
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
> Amended by [ADR 0116](0116-a-saved-file-is-written-whole-or-not-at-all-and-a-failed-write-says-whether-its-path-or-its-storage-failed.md): an Export's failed write raises Collie's `path_not_writable` (`program`) or `file_write_failed` (`unavailable`), neither retryable.

## Considered Options

- **Pull only** (Spider, Crawl4AI): rejected. Handlers already run in the program, and Records outside run state would miss Checkpoints, Sinks and the Console.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
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
  - [Hooks and injectable components in the SDK API](https://github.com/goodfetch/goodfetch/issues/481) (ADR 0104) shapes the "Record before commit" Hook and SDK-handler Sinks.
  - [Testing a user's crawl or automation](https://github.com/goodfetch/goodfetch/issues/482) records what a test handler pushed.
    > Amended by [ADR 0105](0105-a-programs-tests-replay-a-response-recording-in-a-test-engine-that-reaches-no-site.md): `runHandler`'s `AttemptResult.records(dataset)` returns each Record the attempt pushed, as its Dataset's schema parses it, even when a crawl would discard the attempt; a Record's committed form and what Hooks drop or reject are read with `run.records()` and `run.rejectedRecords()` from a crawl started on a Response recording.
- **The `prototype/sdk-api` sketch** holds these signatures in its `output` files for each language, with their snippets.
