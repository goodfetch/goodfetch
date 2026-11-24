# A page's fields are read in one call on every tier, and a push takes one Record or several

A program reads many fields of a page with one call, `page.read(fields)`, an operation of Collie's Page that every fetch tier serves, and pushes one Record or a list of Records with one `push`. Decided by the owner on 2026-11-10 in the grilling of decision G-01 of the consolidated review: Q2, Q3, Q11, Q12 and Q13, answered "Yes, all four" and "Yes, all ten", each answer accepting the recommendation as put.

Until now a field was one getter and so one round trip (ADR 0085), and a Record one push (ADR 0103). ADR 0085 refused a structured read beside the getters because reads awaited together wait about as long as one. That is true of the waiting and not of the work:

- **A message costs the engine work, however short the wait.** The engine spends about 19,464 instructions on each message it dispatches (`E1.instructions` in `TESTING.md`'s Quality register, first measured on 2026-11-02), so a handler that reads twenty fields spends about 389,000 instructions a page on messages alone, where Spider spends about 400,000 on its whole page (ADR 0142).
- **In the browser each getter is one engine read** (ADR 0140's C2: one CDP command per read), so twenty fields are twenty reads.
- **The one-call read ADR 0085 counted on, `evaluateAll`, runs a page function,** which a document tier does not serve and so escalates to the browser (ADR 0093), so every page of an HTTP crawl would be rendered to be read.
- **No getter is built.** The catalog's page operations hold none of ADR 0085's getters, so nothing is undone. The code-free crawl already reads many fields of a page in one pass, with a declaration of its own.

## Decision

- **`page.read(fields)` is an operation of Collie's Page,** for a handler's `ctx.page` and a Collie program's page alike, on every fetch tier. It is one message and, in the browser, one engine read, whatever the number of fields.
- **A field** is declared as the code-free crawl declares one: a CSS or XPath locator, an optional attribute, an optional pattern with its group, and a transform to text, a number, a date or a date and time. It reads the first element its locator finds. A field may also be **a list**, which reads every element its locator finds, or **rows**: a base locator and a set of fields, read under each element the base locator finds.
- **Each field answers its own outcome** (ADR 0079): its value, or null with the reason it was not read. A row that lacks a field keeps its other fields.
- **The call never waits and never escalates.** It is a query (ADR 0099): it reads the document as it is, takes no `timeout`, and an absent element is an answer. A handler that needs an element to appear waits for it with a locator first, which on a document tier is what escalates (ADR 0093).
- **One reader, one declaration.** The code-free crawl's `extraction.fields` takes the same declaration, lists and rows included, and both are read by one reader in `goodfetch-extractor`. Each tier keeps finding the elements where its document lives.
- **The getters stay** as ADR 0085 lists them: Playwright's names for a read of one element at the moment of a Gesture. `evaluateAll` and `page.formats` are unchanged.
- **`push` takes one Record or a list.** A list is one message. Each Record is checked as one is today; the valid ones are staged; the error raised at that `push` names each failing Record by its position and each issue by its path. `push` stays a checked round trip: no SDK holds a Record back to send it with the handler's answer.

## Considered Options

- **Getters only, awaited together** (ADR 0085's reads run concurrently): rejected. Twenty fields are twenty messages and, in the browser, twenty engine reads a page.
- **`evaluateAll` as the one-call read:** rejected. It needs the browser, so every page of an HTTP crawl would be rendered to be read.
- **Fields declared on the Label, read before the handler is called and sent with its request:** not built. It saves one message a page, fixes the fields per Label and gives a Collie program nothing. Nothing here closes it: it would use the same reader and the same declaration.
- **Pushes held in the SDK and sent with the handler's answer:** rejected. A bad Record would be found after the handler returned: it could not be caught, the request would fail for good and its good Records would be lost (ADR 0103).
- **A selector API beside the page** (Parsel- or Cheerio-like): still rejected (ADR 0093). `read` is a member of the page and takes the page's own locators.

## Consequences

- **Amends ADR 0085.** "No row-wise structured read is added" and the rejection of "A structured row-wise read beside the getters" are reversed; a note sits at each. The getters and their rule (one round trip whatever the number of matches) stand.
- **Amends ADR 0093.** A handler reads its page's fields with `ctx.page.read(fields)`; "no selector API beside the page" stands, with a note that `read` is on the page.
- **Amends ADR 0103.** `ctx.push(dataset, record)` also takes a list. On the wire `crawl_request.push` carries the Records of one call and answers one outcome per Record. A note sits at its `ctx.push` bullet and at "A Record that fails raises from `push`".
- **Amends ADR 0140, with no cap raised and no owner module changed.** A `page.read` on a document tier counts in F1, with no selector compilation per page; in the browser it is one engine read of C2. The field reader moves from `goodfetch-crawl` into F1's owner module, `goodfetch-extractor`. A note sits under its table of Shared paths, and the Quality register gains two rows not run yet: `C2.read_commands` and `E1.handled_page_wire_share` (ADR 0024's note of 2026-11-10). `C2.read_commands` reads 2 on today's read path for Exception X10's cause, every engine read making its isolated world (`crates/goodfetch-page/src/driver.rs:1512-1535#read_in_frame`); naming it in X10 is the owner's.
- **The catalog** gains `page.read`, a query marked `collie`, and the field declaration becomes a type of Collie's part of the protocol. Nothing is published, so no format changes version (ADR 0066).
- **Area 6** builds the operation and moves the reader; **area 11** builds each SDK's member, typed from the declaration, TypeScript first (ADR 0099's note of 2026-11-10). `docs/agents/rollout-order.md` places the tickets.
  > Note (2026-11-23, the owner's decision of that day, [ADR 0136](0136-each-implement-spec-run-is-one-whole-spec-and-milestones-are-ticket-metadata-and-owner-checkpoints.md)'s note): the ticket graph places the tickets, not `docs/agents/rollout-order.md`.
- **Not decided here:** whether the `records` field of a handler's answer stays on the wire is the owner's, at the pending review of ADR 0103's note on it; no SDK member fills it.
- **Glossary.** No term changes.

## Existing code

Read at `094c96b2d`.

- Collie's operations, which a test holds in the catalog's order, include no getter: no `textContent`, `innerText`, `getAttribute` or `inputValue` entry. `crates/goodfetch-protocol/src/entries.rs:164-254#COLLIE`
- The code-free crawl declares a field as a CSS or XPath locator, an optional attribute, an optional `match` with its group, and a Value transform to text, a number, a date or a date and time, reading the first element its locator finds. `crates/goodfetch-protocol/src/foxhound/crawl.rs:233-258#ExtractionField` `crates/goodfetch-protocol/src/foxhound/crawl.rs:260-280#FieldLocator` `crates/goodfetch-protocol/src/foxhound/crawl.rs:282-295#FieldMatch` `crates/goodfetch-protocol/src/foxhound/crawl.rs:301-340#FieldTransform`
- Its reader lives in the crawl crate. `crates/foxhound/goodfetch-crawl/src/code_free/fields.rs:63#Reader`
- `crawl_request.push` takes one Record. `crates/goodfetch-protocol/src/foxhound/dataset.rs:44-65#CrawlRequestPushParams`
- `E1.instructions` reads 19464.21 instructions per dispatched message. `TESTING.md:1411`
