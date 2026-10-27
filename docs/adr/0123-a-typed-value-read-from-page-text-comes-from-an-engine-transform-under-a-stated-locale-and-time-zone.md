# A typed value read from page text comes from an engine transform under a stated locale and time zone

When an extracted field is a number, an amount's currency or a date, and the page writes it for people ("1.099,98 €", "01/02/2026", "3 hours ago"), the engine turns the text into the value. It uses one of a closed list of **Value transforms**, and each locale-dependent transform states its locale and, where it needs one, its time zone. The model picks the text and the transform, never the value, and the one table it writes, a `map`'s, waits for a person. Grounding runs the transform again, so a value is vouched for by the engine (ADR 0013). A Plan never replays a parse under a locale the page no longer uses. Provenance keeps the text beside the transform. This answers row D05 of the [pitfall catalog](https://github.com/goodfetch/goodfetch/blob/prototype/sdk-api/sdk-api-sketch/pitfalls.md), from items 4.C5 and 4.C6 of the [pitfalls research](https://github.com/goodfetch/goodfetch/blob/research/common-pitfalls/rnd/research/2026-10-16-common-pitfalls.md). Decided in [Locale-dependent values under Grounding: dates, numbers and relative times](https://github.com/goodfetch/goodfetch/issues/554), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461), every answer by the agent on the owner's delegation. Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

**What was left open.**
- ADR 0013 lets a value "follow from a declared transform".
- [ADR 0091](0091-collies-ai-verbs-live-on-the-page-replay-from-a-plan-file-the-run-names-and-expect-runs-a-success-check.md) keeps, per compiled field, "a transform from a closed, deterministic list".
- No ADR listed the transforms, or said in which locale or time zone they read.
- [ADR 0021](0021-standard-access-by-default-and-escalation-by-new-session.md) matches a Session's language and time zone to its proxy exit, so one page can print "1,099.98" for one Session and "1.099,98" for another.

**What the leaders do.**
- **Crawl frameworks leave parsing to the user's code.** Scrapy's item loaders take any function, and their built-in processors parse no locale ([itemloaders](https://itemloaders.readthedocs.io/en/latest/processors.html)).
- **AI extractors let the model write the value.** Stagehand parses the model's answer with the program's Zod schema (golden scenario 02).
- **Parsing libraries take the locale as input, or guess.**
  - Babel's `parse_decimal('1.099,98', locale='de')` is 1099.98 ([Babel](https://babel.pocoo.org/en/latest/numbers.html)).
  - Zyte's price-parser guesses the separator, and its README asks for `decimal_separator` "to prevent price-parser from guessing the wrong decimal separator symbol" ([price-parser](https://github.com/scrapinghub/price-parser)).
  - dateparser takes the machine's zone (`TIMEZONE` "defaults to local timezone") and anchors relative dates at "the current date and time" ([dateparser](https://dateparser.readthedocs.io/en/latest/settings.html)).
- **Extraction services keep the page's text beside the parsed value.** Diffbot returns `offerPriceDetails` with `amount` and the "full `text`" ([Diffbot](https://www.diffbot.com/docs/extract/product)). Zyte API returns `"price": "51.77", "currency": "GBP", "currencyRaw": "£"` for scenario 02's own page ([Zyte API](https://docs.zyte.com/zyte-api/usage/extract/index.html)).

## Decision

### The engine parses, the model chooses

- **The engine runs the transform, and its result is the value (G1, ADR 0013).**
  - In compiled mode the model writes each field's Value transform once, and the Plan keeps it (ADR 0091).
  - In model mode, and for an Agent's output ([ADR 0100](0100-collies-agent-is-one-bounded-call-on-a-page-judged-by-the-programs-success-check-and-replayed-whole-from-the-plan.md)), the model answers each field with the page text it read and the transform. The engine checks that the text appears in the page, then runs the transform.
  - A model never writes a typed value it parsed itself.
- **The way down is the program's own parse (G8).** A field typed as text on the wire and parsed by the program's own schema (a Zod codec, a Pydantic validator, a serde `Deserialize`) works as [ADR 0082](0082-sdk-schemas-come-from-each-languages-own-schema-types-payloads-are-typed-per-label-and-results-keep-what-the-sdk-does-not-know.md)'s wire side says. Grounding then checks that text.

### The closed list

| Transform | Reads | Writes |
|---|---|---|
| `text` | the element's text, whitespace normalized as `expect` normalizes it, or one attribute; an `href` or `src` is resolved against the document's base URL, as ADR 0093's links are | the text |
| `match` | a capture group of a regular expression over that text, matched in the engine (ADR 0081) | the text |
| `number` | an integer or decimal under its locale's CLDR decimal patterns, or an amount under its currency patterns, whose currency sign it skips ("1.099,98 €" under `de-DE`) | the canonical decimal |
| `currency` | a currency symbol or code under its locale's CLDR names, alone or in an amount under its currency patterns (`€`; `$` is USD under `en-US` and CAD under `en-CA`) | the ISO 4217 code |
| `date` | a date in one of its locale's CLDR date formats, or in a stated CLDR pattern such as `dd/MM/yyyy` | `YYYY-MM-DD` |
| `date_time` | a date and time, likewise, with its time zone | RFC 3339, with its offset |
| `relative_time` | CLDR's relative forms in its locale ("3 hours ago", "yesterday", "il y a 3 heures"), with its time zone | a date-time, or a date where the field is a date |
| `map` | an exact text, looked up in a table the transform declares ("Three" → 3) | the table's value |

- **A field keeps one transform, optionally preceded by a `match`.** The `match`'s capture group is the text the transform reads, so "Price: 1.099,98 € incl. VAT" is a `match` then a `number`, and the price and its currency are two fields read from one element. No longer chain exists. In model mode the answer names the same pair, and Grounding checks the element's text, then runs both.
- **Machine-readable values come first.** `number`, `date` and `date_time` read from a `<time datetime>`, a `<data value>` or a `content` attribute use the invariant forms (ISO 8601, a plain decimal with a dot), whatever the locale, and the compile prefers them. HTML gives these attributes "a machine-readable form" of their element's contents ([HTML](https://html.spec.whatwg.org/multipage/text-level-semantics.html#the-time-element)).
- **Output forms.**
  - A `number` never passes through a float. It writes a JSON number where the field's JSON Schema takes a number, and text where it takes a string, the exact decimal of ADR 0082 (scenario 02's `"51.77"`).
  - A field whose JSON Schema neither type nor format accepts the transform's output is never coerced: the answer that wrote that transform fails Grounding and is fed back through ADR 0012's bounded retry, as any ungrounded answer is.
- **A `map` is the one transform whose table the model writes, so a person checks the table (G1).**
  - Grounding vouches for its source text, which must be one of the table's keys exactly. The table's values are the model's.
  - It is offered only in compiled mode, where the Plan keeps the table and its diff shows it. An entry whose `map` table is new or changed is kept pending for a person ("Needs you", ADR 0031) even after a `verified` Run, never promoted on the Run's Verdict alone, and the Provenance of a value it gave marks the table as not yet accepted.
  - In model mode and in an Agent's output, nothing keeps a table for a person to check, so `map` is not offered there: an answer naming it fails Grounding. A program that needs one declares a text field and maps it in its own schema, the way down (G8).
- **The list is versioned with the Plan file (ADR 0030).** A new transform is added in a minor, under this ADR's rules: deterministic, every parameter stated, recorded in Provenance.

### Strict parsing

- **A text matches one of its locale's forms whole (G1).**
  - Grouping separators may stand only where the locale puts them.
  - An impossible date is never reordered, as dateparser reorders `13/12/10` without `STRICT_DATE_ORDER`.
  - A two-digit year resolves to the century that puts it between 80 years before and 20 years after the fetch time, ICU's rule.
- **The one leniency:** where a locale's grouping separator is a space, any space character (U+0020, U+00A0 or U+202F) matches it, as sites type them.
- **A text that does not parse is not a value.**
  - In compiled mode, a replayed transform that does not parse is Drift of its step: the call gets ADR 0091's one Repair, and after it, or under `plan.frozen`, fails with `plan_drift`; an Agent's task hands over to its live loop instead (ADR 0100). Its `Drift` names the field, the text and the transform.
  - In model mode, the failure is fed back through ADR 0012's bounded retry, and in the end fails with `extraction_invalid`, whose issue names the text and the transform.
  - In an Agent's output, it is an answer that fails Grounding: given back to the model once, then `needs-review` naming the field (ADR 0100). A value Grounded in an Agent tool's result is the program's own data, and no transform reads it.

### Which locale and time zone

- **Every locale-dependent transform states its locale**, a BCP 47 tag. None reads the machine's locale (G2).
- **The setting `extract.locale`** (a BCP 47 tag, `null` by default) fixes the locale for its scope: connection, Run, Session, page or Label (ADR 0083). The compile must use it, and Grounding refuses a transform under any other locale. A tag the engine has no CLDR data for fails with `setting_invalid`, naming the closest one it has.
- **Otherwise the compile chooses the locale** from the page's evidence: the element's language, its currency and the text itself.
  - The element's language is HTML's "language of a node": its nearest `lang`, then the document's default, then the HTTP `Content-Language` ([HTML](https://html.spec.whatwg.org/multipage/dom.html#the-lang-and-xml:lang-attributes)).
  - When the page declares a language, the chosen locale must share its language subtag (`lang="en"` allows `en-GB`, never `de-DE`), and Grounding checks that.
  - A page's `lang` alone cannot be the rule: `en` does not say whether "01/02" is January or February.
  - The Session's `browser.locale` alone cannot be the rule either: it is what the site was asked for, not what it served.
- **Replay never parses under a locale the page no longer uses.**
  - For a Plan entry that holds a locale-dependent transform, the variant's key (ADR 0091's "one variant per page layout", ADR 0100's "one variant per start page layout" for an Agent's task) also holds the inputs its transforms are resolved from: the document's language (its root `lang`, else its `Content-Language`), the Session's `browser.locale` and the `extract.locale` in effect for any locale-dependent transform, and, for a `date_time` or a `relative_time`, the Session's `browser.timezone_id` and the `extract.timezone_id` in effect. All are known before any locator runs.
  - A page served in another language, to a Session of another locale or zone, or under another `extract.locale` or `extract.timezone_id`, finds no variant and compiles a new one once, beside the others, so an `/en/` and a `/de/` page of one layout both replay from the Plan. Under `plan.frozen` a missing variant fails with `plan_drift`, naming the input that changed.
  - Each field also keeps the language of the element it was read from, which is known only once the variant's locators have found the element. A replay that reads a field from an element whose language differs from the one kept, on a page whose document language did not change, is Drift of its step: a per-call entry gets ADR 0091's one Repair, and an Agent's task hands over to its live loop (ADR 0100).
  - The Plan keeps each transform's resolved locale and zone. Since the key and the check hold every input they were resolved from, a replay uses them only where those inputs are the same, and a changed setting never replays the old locale or zone.
  - Strict parsing alone cannot catch "1,099", which is 1099 under `en-US` and 1.099 under `de-DE`.
  - A program that fixes `browser.locale` and `browser.timezone_id` keeps one variant.
- **A date-time that prints no offset takes its time zone (an IANA name) in this order:**
  1. a zone or offset the text itself states, read by the locale's CLDR zone formats;
  2. the setting `extract.timezone_id` (`null` by default), spelled as `browser.timezone_id` is (S1);
  3. the Session's `browser.timezone_id`.

  Under ADR 0021 the Session's zone is matched to the proxy exit, and it is the zone the page's own scripts format times in (Playwright: "this only affects the browser timezone and locale, not the test runner timezone"). It is never read from the engine's machine, and never chosen by the model, since nothing on the page could check a zone the text does not state.
- **A relative time is anchored at the fetch time of the page's document.** That is the moment the engine received its response, read from its `Clock` (ADR 0059) and recorded in Provenance. Under a Response recording it is the recorded time, and for a document a test gives `runHandler` itself, its `Date` header, as ADR 0105 starts that page's clock, so a test reads the same value on every replay. The result is the anchor minus the amount, never rounded, and Provenance marks it relative with its anchor.

### Provenance

- **For each field, Provenance keeps (G6):**
  - the text exactly as read;
  - the transform with its resolved locale, time zone, pattern and anchor;
  - the step it was read in, and in compiled mode the locator it was read with.

  It sits where ADR 0091 keeps the rest of a value's Provenance, in the Run trace, beside what ADR 0100 keeps per output field. A Record reaches it through the crawl request that produced it (ADR 0103).

  > Note (agent decision, pending owner review), 2026-10-26, [#972](https://github.com/goodfetch/goodfetch/issues/972): each committed Record also carries its own Provenance, which `run.records` reads as the Record's `provenance`: `request_id` (its crawl request), `tier` (`tier`, `chosen`: `forced`, `compared`, `sample`, `escalation` or `race`, `forced_by`: `run` or `label`, and the escalating `call`), `encoding` (`encoding` and its `source`, ADR 0119), `origin` (`response_cache` with `fetched_at`, or `response_recording` with `recorded_at`), `clean_up_rules` (the digest the Run holds to, ADR 0047), `wall` (`kind` and `vendor`, ADR 0052), `payment` (`scheme`, `amount`, `currency`, `payee_domain`, ADR 0045) and `value_transforms` (per field: its JSON Pointer, `text`, `transform`, `locale`, `time_zone`, `pattern` and `anchor`). The types are `goodfetch-protocol`'s `foxhound::provenance`; each is absent where the page met none. The engine fills the request, the tier, the encoding and the Clean-up rules today; the Response cache and recording origins, the Wall marker, the payment and the Value transforms are filled by the tickets that build them (#757, #779, #1002 and the cache and recording tickets). A Record, its Provenance and a rejected Record are written with every value the Run resolved replaced by its `%name%` under the Mask's `Kept` scope (ADR 0092), so a secret never lands in a Dataset.

  > Note (agent decision, pending owner review), 2026-10-26, [#1002](https://github.com/goodfetch/goodfetch/issues/1002): a code-free crawl's `fields` fill `value_transforms`, one entry per transform a field ran (a `match`, then its transform, each with the text it read), and a field its page did not give is null with its reason in the Record's `unread_fields`. Until the Value transforms' parsers land (#844, #848), `number`, `date` and `date_time` read only a machine-readable value (`<time datetime>`, `<data value>`, a `content` attribute, or one of those attributes named) in its invariant form, whatever the locale, and a text printed for people is `not_parsed`; a `number` is written as a JSON number only when one holds it exactly. The pure functions are `goodfetch-crawl`'s `code_free::fields`, which #844 moves onto `goodfetch-value-transform`.
- **The output stays the schema's type (S2, ADR 0082).** No value is wrapped with its text. A program that wants the page's text in its data declares a text field of its own, read with `text`, as Diffbot's `text` and Zyte's `currencyRaw` sit beside the parsed value.
- **Failures carry the text.** A `plan_drift` or an `extraction_invalid` from a parse names the text that did not parse and the transform.

## Considered Options

- **The program's own native rules as the only path (ADR 0082):** rejected as the main path, and kept as the way down. Every program would write locale parsing, as every Scrapy project does, and a field typed `number` would have no Grounded path at all.
- **The model writes the value, and Grounding accepts it when one of its formatted forms appears on the page:** rejected by G1. "01/02/2026" is both formatted dates, so a wrong date passes. One value has many printed forms (grouping or none, a no-break space, the symbol before or after), so the comparison is either loose or flaky.
- **A model-written `map` table promoted like any compiled step, and offered in model mode:** rejected by G1. Grounding checks only that the text is one of the table's keys, so "Three" → 4 passes it, and a Run that only extracts can be `verified` with nothing checking the table.
- **Lenient parsing that guesses the separators or the date order,** as price-parser and dateparser do by default: rejected by G1. Their own documentation asks for a hint to stop the wrong guess.
- **The page's `lang`, or the Session's `browser.locale`, as the locale at every call:** rejected. A language without a region leaves the date order open, and the Session's locale is what was asked for, not what was served.
- **A locale keyword in the schema,** per field: rejected. It would add a Goodfetch dialect to every schema library (G7), and a locale belongs to a site, which a setting's scope fits.
- **The machine's locale and zone,** dateparser's default: rejected. It is the pitfall itself: a naive datetime silently takes the crawler machine's zone.
- **A zone chosen by the model:** rejected. Nothing on the page checks a zone the text does not state.
- **The read time as a relative time's anchor:** rejected. It changes on every replay of a Response recording.
- **Each value wrapped with its source text in the output:** rejected by S2. The output is the schema's type, and a program that wants the text declares it.
- **A per-call `locale` argument on `extract`:** rejected by S2. A locale belongs to a site, and the page and Label scopes already carry settings (ADR 0083).

## Consequences

- **Amends ADR 0013.** A "declared transform" is a Value transform from this ADR's list, run by the engine. A model never writes a typed value it parsed itself. The one value a model writes is in a compiled `map`'s table, which a person accepts before the Plan keeps it. A note sits under its first paragraph.
- **Amends ADR 0091.**
  - The closed list of compiled transforms is this ADR's, and a field's transform may be preceded by a `match`.
  - In model mode, the model answers each field with its text and transform, and the engine computes the value.
  - A replayed transform that does not parse, or a field read from an element in another language than the one kept, is Drift, which `GLOSSARY.md`'s **Drift** now covers beside a target that no longer matches its Target fingerprint.
  - The variant key of an entry holding a locale-dependent transform also holds the document's language, the Session's locale and the `extract.locale` in effect, and for a date-time or relative time the Session's zone and `extract.timezone_id`; each field keeps its element's language, checked once the element is found. Under `plan.frozen` a missing variant fails with `plan_drift`.
  - An entry whose `map` table is new or changed is kept pending for a person even after a `verified` Run.
  - `extract.locale` and `extract.timezone_id` join `extract.mode`.
  - Notes sit under its Plan section, its promotion rule and Compiled and model extraction.
- **Amends ADR 0100.** An Agent's output fields follow the same transforms and the same answer form, except `map`, which is not offered there. Its entry's start page variant also holds the start page's document language, the Session's locale and the `extract.locale` in effect, and for a date-time or relative time the Session's zone and `extract.timezone_id`, and each output field keeps the language of the element it was read from, so a replay that reads one from an element in another language is Drift and the live loop takes over. Notes sit under its Grounding bullet and its Plan entry's key.
- **Adds to ADR 0083.** `extract.locale` and `extract.timezone_id` are new settings, `null` by default, set at any scope that takes `extract.*`. A locale tag the engine has no CLDR data for fails with `setting_invalid`, whose message names the closest tag it has.
- **Amends ADR 0117.** `scriptedExtraction(data)` keeps its typed `data`. The testing kit answers in the model's answer form: for each value it finds the page text from which a Value transform under the locale the compile would use (`extract.locale` when set) gives that value, and answers with that text and transform under `model`, or with the locators and transform that read it under `compiled`. A date printed "01.02.2026" is found for `2026-02-01`. A note sits under its `scriptedExtraction` bullet, and one under ADR 0105's Scripted model.
- **Adds to ADR 0105.** A relative time read from a Response recording is anchored at the recorded fetch time, and one read from a document a test gives `runHandler` at that document's `Date` header.
- **Adds to ADR 0059.** A relative time's anchor is the document's fetch time from the `Clock`, recorded in Provenance, so its tests need no clock.
- **`GLOSSARY.md`** gains **Value transform**, and **Grounding** names it.
- **`TESTING.md`** gains the Value transform conformance cases.
- **ADR 0037's matrix:** AI-03 and pain 14 (`data-extraction-quality`) cite this ADR.
- **Who builds what.**
  - **Area 9 (AI and agent)** builds the transforms over CLDR data, the model's per-field answer form, the variant key and the Grounding checks. ICU4X formats but does not parse (`icu_decimal` is an "API for formatting basic decimal numbers", crates.io), so the parsers are Goodfetch's, over its locale data.
  - **Area 6 (Collection)** lists this ADR for Records.
  - **Area 2 (Proving ground and conformance)** serves the cases from the Proving ground's "geo and language variants" pages (ADR 0025).
  - Each lists this ADR.
- **The pitfall catalog's D05** closes as Handled. [ADR 0119](0119-every-fetch-tier-decodes-a-crawls-document-as-the-browser-does-a-label-names-the-content-types-its-handler-takes-and-exports-are-utf-8.md) decodes bytes to text before any transform runs.
- **The `prototype/sdk-api` sketch** holds the two settings in its `settings` files for each language, with a snippet reading a German price and date in its `ai` snippets.
