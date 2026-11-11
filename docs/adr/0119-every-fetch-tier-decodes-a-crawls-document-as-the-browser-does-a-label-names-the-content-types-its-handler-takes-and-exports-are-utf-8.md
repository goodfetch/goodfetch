# Every fetch tier decodes a crawl's document as the browser does, a Label names the content types its handler takes, and Exports are UTF-8

The documents of a crawl handler's page are decoded the same way on every fetch tier. The engine follows the HTML standard's declared steps, with Chrome's `<meta>` prescan: a BOM, an encoding the Label forces, the `Content-Type` charset, the `<meta>` prescan, then UTF-8 for a valid UTF-8 body. A non-browser tier never guesses: it escalates the request to the browser instead. The engine also gives the browser the same encoding, so ADR 0020's sample comparison sees one reading. Content codings are decoded as Chrome decodes them, and a body that fails to decode fails as Chrome fails it. A Label lists the content types its handler takes, and any other response ends `skipped` before its body is read. Every text Export is UTF-8.

The [Common pitfalls catalog](https://github.com/goodfetch/goodfetch/issues/547) found four gaps: rows H16, H17, H25 and D04 of [`pitfalls.md`](https://github.com/goodfetch/goodfetch/blob/prototype/sdk-api/sdk-api-sketch/pitfalls.md), items 2.15, 2.18, 2.19 and 4.C4 of its [research](https://github.com/goodfetch/goodfetch/blob/research/common-pitfalls/rnd/research/2026-10-16-common-pitfalls.md). ADR 0037's CO-24 left "detection and forced encoding" to spec. ADR 0020 compares a sample fetched over HTTP with the browser's, so an HTTP decoder that differs from Chrome's would either fail the comparison or store mojibake where both readings agree. [ADR 0093](0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md) sends every 2xx to the handler without saying what its page is for a PDF.

What the browser and the leaders do (read on 2026-10-16):
- **Chrome guesses legacy encodings, but never UTF-8.** With nothing declared, Blink runs Compact Encoding Detection with the URL as a hint. Its [detector](https://github.com/chromium/chromium/blob/main/third_party/blink/renderer/platform/text/text_encoding_detector.cc) "Should return false if the detected encoding is UTF8" on a web page, so an unlabelled UTF-8 page falls back to the "default encoding determined from system locale or TLD".
- **Chrome's content decoding.** It [decodes](https://github.com/chromium/chromium/blob/main/net/filter/filter_source_stream.cc) gzip, deflate, br and zstd in reverse header order, up to 10 nested codings. It passes `identity` or an unknown coding through raw, "expected that user will see malformed / garbage response". An invalid gzip header [fails](https://github.com/chromium/chromium/blob/main/net/filter/gzip_source_stream.cc) with `ERR_CONTENT_DECODING_FAILED`, and deflate is accepted with or without its zlib header.
- **The libraries.** Scrapy's `TextResponse` tries its constructor argument, the BOM, the header, the in-body declaration, then inference. Crawlee tries the header, the `<meta>`, then `suggestResponseEncoding`, then UTF-8, and `forceResponseEncoding` overrides all of them. Both libraries map labels as libraries do, not as WHATWG Encoding does, where `latin1` means windows-1252.
- **Response text.** Playwright's [`Response.text()`](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/client/network.ts) is `content.toString('utf8')`, and WHATWG Fetch's `text()` is a UTF-8 decode. requests and httpx decode by the header's charset.
- **Other content types.** Crawlee's `HttpCrawler` accepts HTML, XML and JSON by default, extended with `additionalMimeTypes`, and fails any other type with `noRetry`.
- **Export encoding.** Scrapy's `FEED_EXPORT_ENCODING` is `utf-8` with no BOM. Apify's dataset API puts a BOM on CSV by default. RFC 8259 §8.1 forbids a BOM on JSON.

Decided in [Decoding a response the same way on every tier](https://github.com/goodfetch/goodfetch/issues/550), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461). Every answer was decided by the agent on the owner's delegation ([grilling record](https://github.com/goodfetch/goodfetch/issues/550#issuecomment-6204439947)), under the principles of [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

## Decision

### A crawl page's documents, on every tier

- **Which documents.** Every main-frame document a crawl handler's page loads: the crawl request's own document, and each document the handler's own `goto` or `reload` loads on that page (ADR 0093 serves them over the attempt's tier). Frames, popups and pages the handler opens are the browser's, as on a Collie page.
- **The order.** Every tier finds the encoding of such a document the same way:
  1. a BOM;
  2. `fetch.encoding`, when set, for the crawl request's own document;
  3. the `Content-Type` charset, when WHATWG Encoding knows its label;
  4. the `<meta>` prescan as Chrome runs it: the first 1,024 bytes always, then on while only tags allowed in the head appear (Blink's `HTMLMetaCharsetParser`), with the HTML standard's labels, so a UTF-16 label reads as UTF-8 and `x-user-defined` as windows-1252. A declaration later in the document is not read, and nothing is decoded twice, as in Chrome;
  5. UTF-8, when nothing above applies and the whole body is valid UTF-8 (ASCII included).

  Every label maps as WHATWG Encoding maps it, so `latin1`, `iso-8859-1` and `ascii` are windows-1252. A byte that does not decode becomes U+FFFD, as in a browser.

  > Note (agent decision, pending owner review), 2026-10-25, [#764](https://github.com/goodfetch/goodfetch/issues/764): step 4 follows Blink's `TextResourceDecoder` and `HTMLMetaCharsetParser` at the pinned Chromium (156.0.8078.4). The prescan tokenizes the bytes as Latin-1 with the HTML standard's tokenizer, reads `title`, `textarea`, `script`, `style` and the other raw-text elements as text, and checks the 1,024-byte limit as each token ends, as Blink does. For `text/html`, an XML declaration at the very start (`<?xml … encoding="…"?>`) counts when no `<meta>` names a known label. For an XML type, `application/xhtml+xml` included, the XML declaration alone takes the prescan's place, and an XML type that declares nothing follows step 5 like HTML: UTF-8 when valid, else it escalates. A declaration in the document's own bytes reads a UTF-16 label as UTF-8, as Blink's `ClosestByteBasedEquivalent` does, and Provenance names it `meta`, whether a `<meta>` or an XML declaration declared it. Blink's detection of a UTF-16 XML declaration without a BOM is not copied. A handler's own navigation to a document that only a guess reads is a call its tier cannot serve exactly: it escalates at `page.goto` (or `reload`, `go_back`, `go_forward`) under `auto` and fails with `browser_required` naming `fetch.tier` under a forced tier, since `fetch.encoding` does not apply to it. The crawl request's own document that escalates is recorded in its `fetch_tier` Provenance entry with `call: fetch.encoding` and `encoding_source: detected`.
- **No guess on a non-browser tier.** A body that matches none of these steps and holds bytes beyond ASCII can only be guessed. A guess is not exact, so under ADR 0093's rule ("a call is served only when its result is exact") the non-browser tiers do not serve it. Those tiers are `http`, `browser_fetch`, `discovered_api` and `light_browser`.
  - Under `fetch.tier: auto`, the request escalates to `browser`, where Chrome's own detector decides.
  - Under a forced tier, it fails with `browser_required`, whose `fix` names `fetch.encoding` and `fetch.tier` (G6).
  - A frequency detector on the HTTP tier (chardetng, Firefox's) was rejected. It is not Chrome's, so it diverges exactly on the pages where a guess is needed.
- **The browser gets the same encoding.** A declaration the prescan finds counts as declared. At its interception of each such document, the engine sets the `Content-Type` charset to `fetch.encoding` when one applies. With nothing declared and a valid UTF-8 body, it sets UTF-8. Chrome then decodes as every other tier does: a BOM still wins and a declaration is still read. An unlabelled UTF-8 page no longer becomes Chrome's locale default, which is mojibake (G1). A Collie program's own page is not touched and decodes as Chrome does (S3). The rewrite is the engine's own: `ctx.response.headers()` and the Response recording keep the site's headers as received, and Provenance's `setting` or `utf8` says why the encoding differs from them.
- **`fetch.encoding`** forces an encoding: a WHATWG Encoding label, or none (the built-in), at any scope down to a Label. An unknown label is refused with `setting_invalid` when its scope opens (G3, ADR 0083). It stands where the HTML standard's user override stands, after the BOM. It applies to the crawl request's own document on every attempt and tier, not to a handler's own navigations elsewhere. Like every `fetch.*` key, it is Foxhound's (ADR 0093).
- **Provenance and the Run trace** record the encoding and where it came from: `bom`, `setting`, `header`, `meta`, `utf8` or `detected` (the browser's guess) (G6).
- **The sample comparison checks it.** ADR 0020's comparison also fails when the browser sample's `document.characterSet` differs from the encoding the other tier used. A Label on a domain therefore stays on the browser wherever Chrome reads its pages differently from this rule (G1).

### `Response.text()` and `json()`, in both Products

- **`text()` decodes with the response's own encoding,** keeping Playwright's name.
  - A navigation's document response (a crawl handler's `ctx.response`, the `Response` that `goto` returns) uses the encoding its document was decoded with, so `text()` and `page.content()` agree.
  - Any other response uses its BOM, then its `Content-Type` charset, then the `<meta>` prescan for HTML or the XML declaration for an XML type, then UTF-8.
  - It never raises on bad bytes, which become U+FFFD.

  Here the leaders disagree: Playwright and WHATWG Fetch decode UTF-8 always, while requests and httpx honour the charset. S3 then has the deciding ticket pick with evidence, and G1 picks the charset: on a page that declares another encoding, a UTF-8 decode returns mojibake, which `page.content()` would contradict. The name stays Playwright's because the meaning, the body as text, is the same, and the behaviour differs only where Playwright's result is wrong. The docs say so beside the method, since a behaviour changed under an unchanged name is S3's hardest case.
- **`json()` decodes UTF-8** whatever the charset, as RFC 8259 requires and Fetch does, and ignores a leading BOM.
- **`body()`** is the way down: the bytes after content decoding (G8).

### Content codings, on every tier

- **As Chrome decodes them:**
  - the codings `Content-Encoding` lists are decoded in reverse order (RFC 9110 §8.4): gzip, deflate (zlib-wrapped or raw), br and zstd, up to 10;
  - `identity` or an unknown coding passes the body through as it is. That is Chrome's rule, and it covers the common misconfiguration `Content-Encoding: utf-8` on a plain body.
    > Note (agent decision, pending owner review), 2026-10-23, [#759](https://github.com/goodfetch/goodfetch/issues/759): as Chrome's `URLRequestHttpJob` builds its filters, a list naming `identity` or an unknown coding beside known ones, such as `gzip, utf-8`, passes the whole body through undecoded, and bytes after a gzip or deflate stream's end are ignored, as Chrome's gzip filter ignores them. The `http` tier asks for `gzip, deflate, br, zstd`, Chrome's `Accept-Encoding`.

  Every tier then holds the same bytes, so the comparison sees one result.
- **A listed coding that fails to decode** fails as Chrome fails it, on every tier: ADR 0085's `navigation_failed`, carrying the network error `net::ERR_CONTENT_DECODING_FAILED`. It is retryable, so a crawl retries it under `crawl.retry.max` (ADR 0093). A body labelled gzip that is plain is therefore a failed request, never a page of garbage.
- **A body still compressed.** A text type whose decoded body still begins with gzip's or zstd's magic bytes fails the same way on every tier, where Chrome would show garbage (G1). That is a body compressed twice and labelled once. The text types are `text/*`, `application/json`, `application/xml`, `application/xhtml+xml`, `application/javascript` and any `+json` or `+xml` type.
- **A Proving ground family for it.** ADR 0025's Pages list gains content-coding pages, which serve:
  - a body labelled gzip that is plain;
  - an invalid gzip header;
  - `gzip, br`;
  - raw deflate;
  - an unknown coding (`utf-8`);
  - a body compressed twice with gzip and with zstd but labelled once.
- **What it does not cover.** A compressed body that was cut is [ADR 0118](0118-every-fetch-tier-follows-redirects-as-the-browser-does-fails-a-cut-body-and-resends-only-what-a-site-never-processed.md)'s `response_truncated`, not a decoding failure. A `page.request` whose body fails to decode raises `navigation_failed` with `net::ERR_CONTENT_DECODING_FAILED`, as ADR 0118 ends its other transport failures. The decoded size stays bounded by ADR 0067's decompression-bomb limits.

### Responses that are not an HTML document

- **`crawl.content_types`** lists the media types a Label's handler takes, as `type/subtype`, `type/*` or `*/*`. The built-in is `text/html` and `application/xhtml+xml`, and it can be set at any scope down to a Label.
- **The type** is the response's MIME type as the WHATWG MIME Sniffing standard computes it for a navigation, so a missing `Content-Type` is sniffed as Chrome sniffs it.
  > Note, 2026-10-25, [#764](https://github.com/goodfetch/goodfetch/issues/764): Chromium's own MIME sniffer (`net/base/mime_sniffer.cc`, over at most the first 1,024 bytes, `kMaxBytesToSniff`), not the WHATWG MIME Sniffing standard's algorithm with its 1,445-byte resource header, is the reference; a conformance case per pinned Chromium build checks it.
- **Which responses it checks.** Every response that would reach the handler: a 2xx, or a status in `crawl.handle_statuses`. It runs after ADR 0093's status rules and Block causes.
- **A type not listed** ends the request `skipped` with the reason `content_type`, and a message naming the type. Its body is never read, so a 50 MB archive costs only its headers. It is not retried and never reaches the handler. A skip is an expected outcome, not a failure (G1). Crawlee fails such a request instead.
- **An accepted type that is not a document.** Only HTML and XHTML are documents. For any other accepted type (a PDF, JSON, XML, an image):
  - on every tier, `ctx.response` holds the body (ADR 0093 always keeps it), within ADR 0067's limits on body size and decompression, and the page holds no document;
  - a call that reads, waits on or acts on that document fails at once with `response_not_document`: a locator, `content()`, `evaluate`, a screenshot or an AI verb;
  - `response_not_document` is a `ProgramError`, not retryable, and names the type. Its `fix` says to read `ctx.response` (G6);
  - the call never escalates, since the browser would hold no document either;
  - `page.request`, `goto` and the context's own members work as usual.
  > Amended on 2026-11-11 by the owner (grilling of decision G-19 of the consolidated review, Q4 to Q9 answered « Yes to all », each on the grilling's recommended option; the questions are quoted as put, and the owner did not read this note's other words). Q4 was put as reopening line 78 ("Only HTML and XHTML are documents"), and the owner reversed it for these files: a PDF or an Office file becomes something Goodfetch reads as text and Markdown, built in, behind the Label's existing content-type setting. As put: "PDF with a text layer; Word (.docx, .doc), Excel (.xlsx, .xls), PowerPoint (.pptx), OpenDocument (.odt, .ods, .odp), RTF; tables as Markdown tables, headings where the file marks them; locators, actions and screenshots still refuse; a crawl still skips a PDF unless its Label lists the type; converters inside the engine, from libraries under licences compatible with Goodfetch's, chosen when the spec is written" (Q4). OCR is its own setting, off by default: "a local model, downloaded once at a pinned version and checked by checksum, no paid service; off loads nothing" (Q5). E-books, emails with attachments, archives and source code are taken, "each a content type a Label lists, built after PDF and Office", archives within the existing decompression limits (Q6). A user converts a file they already have with "one operation in the Operation catalog (so CLI, MCP, SDKs and the HTTP listener get it), taking the file itself; crawls keep refusing `file://`" (Q7). "The existing Extractor Plugin kind names the content types it takes" (Q8, ADR 0033's note of that day). The path check as put: "an HTML page goes through exactly what it does today; a document page gets one parse in place of the HTML parse, on Page analysis (F1), owner `goodfetch-extractor` unchanged, no limit raised; the spec adds a register row for a document page; with the setting off the body is never read" (Q9). Their rows are CO-76 to CO-82 of ADR 0037's matrix, and RT-33.
  >
  > Note (the grilling session's own, 2026-11-11, same grilling, awaiting the owner's reading): these files are not documents in this ADR's sense, since they have no DOM. Their text and Markdown are read through `formats`; every other call line 80 names (a locator, `content()`, `evaluate`, a screenshot, an AI verb) still fails with `response_not_document`, whose fix is to name `formats` too. As built, `formats` is among the calls that fail on such a page, and the fix names only `ctx.response` (`crates/goodfetch-protocol/src/foxhound/fetch.rs:245-252#RESPONSE_NOT_DOCUMENT`; `formats` is a `Read` call, `crates/foxhound/goodfetch-fetch-tier/src/served_calls.rs:58` (`(&PAGE_FORMATS, Row::Read)`) and `:110-114#Row`, and those fail on a page with no document, `:217-224#reads_document`; read at `72ce9ba96b4468a40b03dcb14f40acd7b606f737`): the spec of the order of work's "Collection's later parts" line (`docs/agents/rollout-order.md`, step 16) builds the change.
- **On the browser tier,** the engine reads the response at its interception and ends the navigation there, so Chrome never starts a download from it. A handler's own `goto` or `reload` to such a response follows the same rule on its page, so no navigation of a crawl page adds a `Download` to its Run.
- **A download a handler's gesture starts** (a click on an `<a download>` link, a button whose script streams a file) is not a navigation, and stays ADR 0085's `Download`, kept with the Run and read through the page's `download` event.
- **A Collie program's own page is unchanged.** Its navigation to a file stays ADR 0085's download, as in Playwright, and how its `goto` reports one is area 5's spec.

  > Note (agent decision, pending owner review), 2026-10-25, [#950](https://github.com/goodfetch/goodfetch/issues/950): on the tiers that fetch the document themselves, the fetch driver judges the type as soon as the response's head names one, and drops the body unread; a response with no `Content-Type` is read whole (within ADR 0067's limits), its content codings decoded, then sniffed. A response with neither a type nor a body, such as a listed redirect, is an empty HTML document, so it reaches the handler. Browser-fetch hands its head over only with its whole body, so on it a refused body has been read by the browser, though never handed on. The handler's request carries its document's `response` (final URL, status, headers as received, encoding and `request_id`), whose body `response.body` reads, stored in the Run's folder only once asked for. A page holding no document teaches the Frontier nothing, is not judged by the Page check, and under `fetch.tier: auto` runs no browser sample, the browser holding no document either. A handler's own `goto` or `reload` on a document tier to a response that is no document leaves its page holding none. Provenance's `fetch_tier` entry names `encoding` and `encoding_source`; a browser run records the `encoding` its navigation's response names, with `detected` only for the guess it escalated for, until the browser's interception (#765) runs this ADR's order itself.

  > Note (agent decision, pending owner review), 2026-10-28, step 7 review (580-582-6), amending #950's note: a response with no `Content-Type` is no longer read whole before it is sniffed. The fetch driver decodes its content codings as the body arrives, with the engine's streaming decoder. Once 1,024 decoded bytes (`kMaxBytesToSniff`) are in, it sniffs them, and a type the Label does not take drops the rest of the body unread, as a type the head names does. A body shorter than that, or one that does not decode, is still judged once it has been read whole. A refused body that would have failed to decode later is therefore skipped for its type and not failed.

  > Note (agent decision, pending owner review), 2026-10-26, [#765](https://github.com/goodfetch/goodfetch/issues/765): the browser tier's rules are a document reader Foxhound registers on a handler's page through the page crate's `HandlerPages` seam, beside the response observer and no part of the page's value, so a Collie program's page has none. It reads each main-frame document's response at the interception, a redirect's excepted, before the navigation gate's judgement. The crawl request's own document is the first one the page's navigation reaches. Its type is checked against `crawl.content_types` (and `crawl.handle_statuses`) from its head, so a refused type is stopped with its body unread and the request ends `skipped` with `content_type`. Any response that is no HTML or XHTML, the crawl's own or one of the handler's own navigations, is read whole, and its navigation is answered with a blank document, so Chrome renders and downloads nothing. The handler's `Response` keeps the site's status and headers, `response.body` reads the bytes kept, and document calls fail with `response_not_document` until a navigation reaches a document again. A document's encoding runs this ADR's order on its body. Where `fetch.encoding` (the crawl request's own document only) or the UTF-8 step decides, the response is answered with the body already read and its `Content-Type` charset set for the browser alone (Chrome ignores a charset set by `Fetch.continueResponse`), the handler's response keeping the site's header. A BOM, a header charset and a `<meta>` the browser reads itself. Provenance's browser `fetch_tier` entry names `document.characterSet` and the source the order found, none where only a guess read it. The sample comparison never matches when the cheaper tier's encoding and the browser's `document.characterSet` differ.

### Exports

- **Every text Export format is UTF-8:** JSON, JSONL, CSV, XML, HTML table, RSS and Atom, and Markdown (ADR 0026). So is every Export file a Sink receives. XLSX, Parquet, Avro and Arrow IPC carry UTF-8 strings as their formats define.
- **JSON and JSONL** write characters as themselves, escaping only what JSON requires. They carry no BOM (RFC 8259).
- **CSV carries no BOM** unless `export.csv.bom` is on (off by default). A BOM ends up in the first column's name in Python's `csv` module, Node's csv-parse and Unix tools. Spreadsheet users have XLSX or this setting.
- **Values are written as decoded,** never normalized and never repaired, so ADR 0073's Unicode record round-trips byte for byte.

## Considered Options

- **The HTML standard's full algorithm with chardetng for the guess:** rejected. chardetng is Firefox's detector, not Chrome's, so the tiers would disagree exactly on undeclared legacy pages, and the comparison would see two readings.
- **A library order (Scrapy, Crawlee):** rejected. Their label mapping (`latin1` as ISO-8859-1) and their treatment of a `<meta>` UTF-16 label differ from every browser's.
- **Leaving Chrome's decoding of an unlabelled UTF-8 page as it is:** rejected by G1. Chrome falls back to its locale's default, deliberately, to push sites to label their pages, and the result is mojibake. The engine's rule reads those pages correctly on every tier.
- **A forced encoding that beats the BOM** (Crawlee's `forceResponseEncoding`): rejected. Chrome cannot be made to ignore a BOM, so the tiers would differ, and a BOM is almost never wrong.
- **`text()` always UTF-8, as Playwright and Fetch:** rejected by G1. `ctx.response.text()` would disagree with `page.content()` on a Shift_JIS page.
- **Every content type to the handler, as Scrapy:** rejected, as ADR 0093 rejected every non-2xx reaching the handler. A PDF would pass as an empty page unless every handler checked, and each one would be downloaded in full (G1, G5).
- **Failing an unaccepted type, as Crawlee:** rejected by G1. Nothing went wrong: the crawl does not collect that type.
- **Treating a non-document response as a download:** rejected. A crawl collects Records from the request's response, and a `Download` belongs to a page's gestures (ADR 0085).
- **JSON and XML as documents on every tier:** rejected for V1. Chrome renders them as viewer documents that the HTTP tier would have to imitate, and `ctx.response.json()` and the program's XML parser read them exactly.
  > Note, 2026-11-11 (the owner, grilling of decision G-19 of the consolidated review, Q22: « Yes to all », on the option as put; the owner did not read this note's wording): the owner reversed this line in part. The engine reads the rows of a CSV and the nodes of an XML feed, "taken as readers of rows and nodes, not documents, off unless a Label lists the type; the rest of ADR 0119:111 stands".
  >
  > Note (the grilling session's own, 2026-11-11, same grilling, awaiting the owner's reading): one Record per row or node; Scrapy and Colly ship readers of CSV rows and XML nodes (ADR 0037's matrix, CO-75).
- **Reading a mislabelled body as it is when it decodes:** rejected. Chrome fails it, so the HTTP tier would serve a page the browser cannot, and the comparison would fail.
- **A new Error code for a decoding failure:** rejected. ADR 0085 already defines `navigation_failed` with the browser's network error, which the browser tier raises for it, so the same code serves every tier.
- **A BOM on CSV by default, as Apify does:** rejected. It breaks programmatic readers, which are what an SDK serves first, and XLSX exists for spreadsheets.
- **Repairing mojibake, as ftfy does:** declined. Repair changes text that Grounding (ADR 0013) requires to be on the page, and the decoding rule above prevents the cause.

## Consequences

- **Amends ADR 0020:** every tier decodes the documents of a crawl handler's page by this ADR's order, and the comparison also fails when the browser sample's `document.characterSet` differs. Content codings are decoded as Chrome decodes them. A note sits under "The engine sets decompression, redirects and timeouts itself".
- **Amends ADR 0093.** Notes sit at each place:
  - **Fetch tiers:** an undeclared document that is not valid UTF-8 escalates from a non-browser tier, and fails with `browser_required` under a forced tier.
  - **Which responses reach a handler:** `crawl.content_types` is checked after the status and Block-cause rules, and `response_not_document` applies to an accepted type that is not a document.
  - **Skip reasons:** `content_type` joins them.
  - **The settings table:** `crawl.content_types` and `fetch.encoding`.
  - **The Error codes table:** `response_not_document`.
- **Amends ADR 0085:** `text()` decodes with the response's encoding, and a body that fails its content coding is `navigation_failed` with `net::ERR_CONTENT_DECODING_FAILED`. A navigation of a crawl handler's page never starts a download, while a download its gesture starts stays a `Download`. Notes sit under "Bodies", "An unreachable site" and "Downloads".
- **Amends ADR 0079:** `content_type` joins the skip reasons. A note sits beside ADR 0107's.
- **Amends ADR 0026 and ADR 0103:** every text Export is UTF-8, and CSV's BOM is the setting `export.csv.bom`, off by default. Notes sit under "Export formats" and under "The format is named".
- **Amends ADR 0025 and ADR 0073:** ADR 0025's Pages list gains the content-coding pages above, and the odd-encodings family and the "PDFs and downloads" pages now have expected results. Each encodings page is read by this ADR's order, with where its encoding came from, and is the same on every tier. A file is `skipped` with `content_type` by default, and with `crawl.content_types` it reaches the handler with its bytes and no Download.
- **Error codes.** `response_not_document` joins `goodfetch-protocol` as a Foxhound code in the `ProgramError` group, not retryable, through `xtask codegen`'s naming checks (ADRs 0079, 0099).
- **`TESTING.md`** gains the response decoding conformance cases.
- **Who builds it:**
  - **Area 2 (Proving ground and conformance)** builds the oracle's expected results for the encodings, PDFs and downloads, and content-coding pages.
  - **Area 5 (Site access)** builds the decoding order on each tier, the browser-tier interception that sets the charset and stops a file's navigation, the content-coding rules and the comparison's encoding check.
  - **Area 6 (Collection)** builds `crawl.content_types`, the `content_type` skip, the page with no document, Provenance's encoding fields and the Export encodings with `export.csv.bom`.
  - **Area 11 (Client surfaces)** builds `text()`'s rule in each SDK, the settings' types and `response_not_document`.

  Each area lists this ADR.
- **ADR 0037's matrix:** CO-24 is now `decided`.
- **The `prototype/sdk-api` sketch** has:
  - the settings, the skip reason and `response_not_document` in each language;
  - `text()`'s rule on `Response`;
  - a snippet with a Label that takes PDFs and one that forces Shift_JIS.

  The pitfall rows H16, H17, H25 and D04 are now Handled.
