# Every surface spells each catalog name by its own rule, shapes come from declared kinds, and `call` is the way down

One Operation catalog generates every surface (ADRs 0011, 0038), and this decides how an entry becomes a TypeScript, Python or Rust member, a CLI command, an MCP tool and the names an error shows.

**The decision in brief.**
- Each entry has one canonical name made of snake_case words, which each language spells by one rule, initialisms included.
- The catalog declares what an entry is, never how a language shows it, so each language's generator applies one form per kind, with no per-entry override and no alias.
- Hand-written members are listed against the entries they serve, so a missing or stale one fails the build.
- A raw `call` is the Client layer's way down.
- Deprecations carry each language's type-level marker.
- The engine names everything in the caller's own spelling.
- Both Products' Connections implement one `Connection` type, and each Rust SDK crate's public API shows only its own Product's entries.

**Evidence.**
- **Playwright** keeps per-language aliases and language-only entries in 461 `* langs:` lines, and playwright-python a table of reasoned exceptions ([browser drivers §4.3](https://github.com/goodfetch/goodfetch/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/browser-drivers.md)). So its languages differ entry by entry: `AbortSignal` is JavaScript-only, and Java renames `goto` and `continue`.
- **Smithy** turns language-neutral traits into each language's idiom.
- **The languages disagree** on initialisms (`HTTPServerError` in PEP 8, `Uuid` in Rust's C-CASE, `waitForURL` in Playwright) and on keywords (PEP 8's trailing underscore, rustc's `r#`).
- **`tsc` never reports a deprecation**, mypy only on request, and MCP has no deprecated field.

These were checked against pinned sources on 2026-10-15 in the [catalog mapping research](https://github.com/goodfetch/goodfetch/blob/research/sdk-catalog-mapping/rnd/research/2026-10-15-sdk-catalog-mapping.md) (CM below), beside the [SDK design guidelines research](https://github.com/goodfetch/goodfetch/blob/research/sdk-design-guidelines/rnd/research/2026-10-14-sdk-design-guidelines.md) (lessons 13 and 30 to 35).

Decided in [Mapping the catalog to each language and surface](https://github.com/goodfetch/goodfetch/issues/471), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461), by the agent on the owner's delegation of 2026-10-14 ([rounds 1 to 3](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6181920227), [grilling record](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6181926808), revised by the [first](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6187903685), [second](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6188281434), [third](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6188876602), [fourth](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6189048777), [fifth](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6189184662), [sixth](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6189288696), [seventh](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6189415526), [eighth](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6189704642), [ninth](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6189914566), [tenth](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6190104229) [eleventh](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6190338232) and [twelfth](https://github.com/goodfetch/goodfetch/issues/471#issuecomment-6190490061) addenda). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

## Decision

### Names

- **One canonical name per entry.**
  - Every operation, parameter, result field, event, Error code and setting key has one name in `goodfetch-protocol`: snake_case ASCII words, each `[a-z][a-z0-9]*`. Word boundaries are thus explicit, as ADR 0038's wire and ADR 0083's keys already are.
    > Amended by [ADR 0132](0132-each-request-may-carry-its-callers-trace-context-across-goodfetchs-own-links-and-the-engines-spans-stay-experimental.md): `_meta`, the Trace context every request's params may hold, is an envelope field, not a catalog parameter: its underscore is outside this rule, and no generator gives it a member.
  - Every surface derives its spelling from these words by its own rule.
  - No entry has a per-language override (S1). When a rule gives a bad result, the canonical name changes for every surface, as ADR 0079 does for a class name that would shadow a built-in.
- **An operation is `<namespace>.<member>`.**
  - The namespace is the value the operation acts on or concerns, singular: `page.goto`, `locator.click`, `run.checkpoint`, `run.start`. Setting keys keep ADR 0083's areas. An operation is called on one value, and one noun per value then reads the same in the SDK, the CLI (`run checkpoint <run>`, as `gh run view`), MCP and errors.
  - An operation that acts on a value takes it as its first parameter, named after the namespace.
- **The value an operation acts on owns it** (G2, S3). An operation whose first parameter is a value the SDK hands out is a method of that value, under its canonical member, and the SDK fills the parameter: `run.checkpoint()`.
- **An operation that acts on no value is a member of the Connection.** Its catalog entry declares the member's name, chosen by the ticket or spec that owns it (`run.start` is `start`).
- **One name means one operation on one value, in every language.** Codegen refuses two entries that would give one receiver the same member, across a Product's whole catalog, since Rust has no overloading (S1). One operation may still take alternative argument shapes: TypeScript overloads, Python's `@overload`, a Rust `impl Into<…>` parameter.
- **Casing** (S1, then S3: one rule per language, which keeps Playwright's names).

  | | TypeScript | Python | Rust |
  |---|---|---|---|
  | Members, parameters, fields | camelCase: `waitForURL`, `baseURL`, `url()` | snake_case: `wait_for_url` | snake_case: `wait_for_url` |
  | Types and classes | PascalCase: `CDPSession`, `JSONValue`, `URLMatcher` | PascalCase: `CDPSession`, `AsyncCDPSession`, `CDPCommandRefusedError` | UpperCamelCase: `CdpSession`, `JsonValue` |
  | Setting keys (ADR 0083) | nested camelCase segments | nested snake_case segments | modules and an UPPER_SNAKE constant |

- **Initialisms.**
  - They come from a closed list kept in `xtask codegen`: `ai`, `api`, `cdp`, `css`, `dom`, `html`, `http`, `https`, `js`, `json`, `mcp`, `ok`, `pdf`, `uri`, `url`, `xml`. `js` and `ok` keep Playwright's `toHaveJSProperty` and `toBeOK` (ADR 0091), while `response.ok()` starts a name and stays lowercase.
  - In TypeScript names and Python class names an initialism is in capitals, except at the start of a camelCase name, where it is lowercase. In Rust it is one word.
  - This follows each language's own leaders: Playwright, the web platform, Stainless (`APIError`, `baseURL`) and `@types/json-schema` (`JSONSchema7`) in TypeScript; PEP 8, the standard library, httpx and Playwright for Python in Python; C-CASE in Rust (CM A3, A4, B1, B2, B6, B8).
  - An abbreviation that is a word stays one: `id` gives `runId`, `timezoneId` and `getByTestId`, as in Playwright.
  - A word joins the list before any released name uses it, since adding it later renames that name.
- **Event names** passed to `on()` are the event's words joined in lowercase in TypeScript and Python (`requestfinished`, `pageerror`, `websocket`), as Playwright and the DOM spell events (CM A4). The wire keeps snake_case, and Rust's marker types are UpperCamelCase (`events::RequestFinished`), as ADR 0081's sketch has them. An event name is a string in TypeScript and Python, so it is data like an enum value (below): one taken from Playwright keeps its spelling, and its entry declares the words of Rust's marker where the event's own would clash with another type, as `events::WebError` for ADR 0085's `pageerror`, named after its `WebError` payload since `PageError` is ADR 0079's group, and `events::NewPage` for the Session's `page`, beside the `Page` type, the names ADR 0085's hand-off and the sketch give them.
- **Enum values are data, not names.** TypeScript and Python keep the wire string as a literal. A value taken from a web standard or from Playwright keeps its spelling (`domcontentloaded`, `no-preference`) and declares its words for Rust's variant (`DomContentLoaded`).
- **Reserved words are escaped by one rule per language** (S1, S3, T).
  - **TypeScript** needs no escape: a reserved word is a valid member name (`route.continue()`). A positional parameter whose name is reserved gets a trailing `_` in the declaration only, which callers never write.
  - **Python** adds a trailing `_` to a hard keyword used as an identifier, as PEP 8 says and Playwright for Python does (`continue_`, `and_`). It applies to methods, keyword arguments and attributes, never to dict keys or string literals. Soft keywords such as `match` and `type` need none.
  - **Rust** uses raw identifiers (`r#continue`, `r#type`, `r#move`), the spelling rustc's own help suggests. prost and aws-sdk-rust generate them, and serde and schemars strip the `r#` on the wire (CM B3 to B5).
  - A future SDK language adds its own escape rule, and may only escape, never rename, unlike Playwright for Java, which renames `goto` and `continue`.
- **Names codegen refuses,** so the catalog renames the entry for every surface:
  - a name Rust cannot make raw (`self`, `super`, `crate`, `Self`);
  - a name a language gives meaning on every value:
    - TypeScript's `then`, which makes a value thenable so that `await` calls it, and `constructor`;
    - Python's dunder names;
    - the methods of the Rust traits every handle implements (`clone`, `eq`, `fmt`, `hash`);
  - `call`, reserved for the raw call;
  - a class name that shadows a built-in (ADR 0079).
- **No aliases** (S2).
  - One name per entry on every surface.
  - The only second names are a deprecated name during its window (ADR 0030) and a leader's tombstone a ticket chose, such as ADR 0080's `waitForTimeout`. Each is typed or marked as below.
- **Prefixes with one meaning,** which codegen checks (S1, T; Azure's standard verbs, guidelines lesson 30):

  | Prefix | Meaning |
  |---|---|
  | `new_` | opens a value the caller closes (`newPage`, `newCDPSession`) |
  | `wait_for_` | a waiter, armed when called (ADR 0080) |
  | `expect_` | Python's block form of a waiter, and nothing else |
  | `is_` | answers yes or no and never raises for "no" (ADR 0079) |
  | `get_by_` | builds a Locator without I/O |
  | `explain_` | reports where values came from (ADR 0083) |
  | `on` | observes side activity (ADR 0081) |

  - A read the SDK answers itself is named for what it returns, never with `get_`, except names kept from Playwright (`getAttribute`, `getBy*`). A lookup by key in a store is `get` and its typed variants `get_<type>`, as `Map.get` and ADR 0103's `get(key)` and `getBytes(key)`.
  - `set_` changes state the value itself holds, as Playwright's gestures (`setContent`, `setInputFiles`) and a Run's progress message do, never a setting (ADR 0083).
  - Other verbs, such as starting or resuming a Run, are their owning ticket's.

### Shapes

- **The catalog declares each entry's kind, and each language has one form per kind** (S1, S3, T). Most rows restate ADRs 0078 to 0083, so that codegen applies them without exceptions:

  | Kind | TypeScript | Python | Rust |
  |---|---|---|---|
  | Round trip | `async` method | `async def` in an `Async…` class, a plain method in its sync twin | method returning a `Call<T>` builder, awaited; `send()` in `blocking` |
  | Round trip that never waits (ADR 0085's questions: `isVisible`, `count`, `all`) | `async` method with no `timeout` option | the same, with no `timeout` | method returning a `Query<T>`, awaited, with no timeout |
  | A read the SDK answers itself, on a handle | method, as all of Playwright's are (`page.url()`, `response.ok()`, `run.id()`, `connection.engine()`) | a read with no arguments is a property, except a name starting `is_` or `as_`, which is a method: Playwright for Python's generator rule (`response.ok`, `page.url`, `run.id`, `page.is_closed()`, CM A2) | method (C-GETTER) |
  | Builder without I/O (`getByRole`, `filter`, `first`) | sync method | sync method; a property when it takes no argument (`locator.first`), by the same generator rule | method |
  | Sub-object without I/O (`keyboard`, `request`, `identities`) | `readonly` property | property | method |
  | Waiter (ADR 0080) | `waitForX(…)` | `wait_for_x(…)`, and an `expect_x` block for an event the program's own action triggers | `wait_for_x(…)` returning a `Waiter` with `during(action)` |
  | Side-activity event (ADR 0081) | `on('x', listener)` returning a `Subscription` | the same | `events(events::X)` returning an `EventStream` |
  | Sequence (ADR 0081) | `AsyncIterable` | async iterator; iterator in the sync API | `EventStream`; `Iterator` in `blocking` |
  | Opener | the value, with `await using` | the value, with `async with` or `with` | the value, with `close()` |
  | Cancellable (ADR 0080) | `signal` option | task cancellation | dropping the future |
  | Duration (ADR 0080) | `number` of milliseconds | `timedelta` | `Duration` |
  | Point in time ([ADR 0092](0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md)) | `Date` | time-zone-aware `datetime` in UTC | `SystemTime` |

  > Note (agent decision, accepted by the owner on 2026-10-21), 2026-10-21, [#1174](https://github.com/goodfetch/goodfetch/issues/1174): each catalog `Operation` declares its `kind` (`OperationKind`), which the catalog file writes as `kind`. The CLI tracer needed three: `round_trip`, `opener` (its result carries the new value's ID, `<namespace>_id`, which the CLI prints) and `sequence` (an operation that answers through notifications while in flight, `events.subscribe` and `page.body`, which is no CLI command until `--follow`). [#1180](https://github.com/goodfetch/goodfetch/issues/1180) adds the other kinds of the table above and the SDK-only and person-only markers.

  > Note (agent decision, accepted by the owner on 2026-10-21), 2026-10-21, [#1180](https://github.com/goodfetch/goodfetch/issues/1180): `OperationKind` adds `query` (a round trip that never waits, so it takes no `timeout`), `read` (a read the SDK answers itself on a handle), `waiter` and `event` (a side activity listened to). Builders and sub-objects without I/O are no catalog operation, so they get no kind; "cancellable" stays the operation's `cancellable` flag, which any kind may carry. Each operation also declares its `reach`, written `reach` in the catalog file: `all_surfaces`, `sdk_only` (ADRs 0089, 0092, 0106) or `person_only` (ADR 0011's list). The CLI offers every `all_surfaces` round trip, query, read and opener as a command and every sequence as a command with `--follow`, and no waiter or event. `catalog::SDK_ONLY` names the SDK-only entries before they exist (`session.cookies`, `session.add_cookies`, `session.clear_cookies`, `page.local_storage`, `page.session_storage`, `page.lend`, `effect.open`, `effect.close`): the CLI offers none of them whatever its marker, and a Quick test fails when the catalog defines one without `sdk_only`. The deprecation keeps `since` and `note`, the note naming the replacement; a structured replacement and earliest removal join with the ticket that renders the SDKs' markers from them.

  > Note (owner's decisions of 2026-11-10, in the grilling of decision G-01 of the consolidated review, Q5 and Q6, answered "Yes, all ten"): the generators learn the kinds one language at a time, TypeScript first; Python's and Rust's follow as tickets of their own. The first `Page` and `Locator` members are emitted by the TypeScript generator, not written by hand and listed in its manifest. A Client conformance case is declared once, with the leader, and lists each surface as its SDK gains the member (`TESTING.md:930`).

- **One form for reads in TypeScript.** A read on a handle is a method whether it may change (`page.url()`) or is fixed at creation (`request.url()`, `run.id()`): one form in the language (S1), Playwright's (S3), which ADR 0085 keeps for the page API. A property would have suited a fixed fact better for T, since a method used as a value can pass unseen in a template string, but T ranks below both.
- **Plain data** (results, error fields, event data that is not a page-API class):
  - TypeScript: `readonly` properties;
  - Python: attributes;
  - Rust: public fields of `#[non_exhaustive]` structs, with the error type staying opaque (ADR 0079).
  - A field only Foxhound fills belongs to a Foxhound type, never to a type both Products share (as `CrawlRun` beside `Run`), since a Rust struct cannot hide a field from one crate.
- **Parameters.**
  - Required parameters are positional, in the catalog's order, and a boolean is never positional.
  - Optional parameters go in a trailing options object in TypeScript, as keyword-only arguments in Python, and as builder methods named after the parameter in Rust.
  - Three states follow ADR 0083.
- **Typed defaults.** Signatures carry no default value, since defaults are settings the engine resolves (ADR 0083). Doc comments state each default in the language's own type: `30_000`, `timedelta(seconds=30)` and `Duration::from_secs(30)` for 30 s.
- **Twins** (S1, S3).
  - Python: every class with a round trip has an `Async<Name>` twin in the same module with the same members, as ADR 0090's `Collie` and `AsyncCollie`, httpx and psycopg have them. Plain data, errors, settings types and `Label` serve both.
  - Rust: `blocking` repeats the async API under the same type names (ADR 0081).
  - TypeScript: async only.
- **ADR 0091's AI verbs and `expect`** follow these rules (S1, S3):
  - `act`, `observe` and `extract` on `Page` and `Locator` are round trips. `variables` is an option, `{ variables }`, `variables=` and `.variables(…)`, on `page.agent` too (ADR 0100). `StepResult` is plain data with `output`, `step` and `usage`. `ariaSnapshot({ mode })` is `aria_snapshot(mode=…)` and Rust's `aria_snapshot().mode(…)`, and `getByRef` is `get_by_ref`.
  - `expect` is one function whose argument, a page, a locator or a response, picks its matchers: TypeScript overloads, Python's `@overload`, and in Rust a generic over an `Expectable` trait, since each target returns its own matchers. A matcher's expected text is `string | RegExp`, `str | Pattern[str]` or Rust's `impl Into<Expected>`.
  - A negated matcher is a kind with one form per language, each the leader's: `.not.toHaveURL()` in TypeScript, `not_to_have_url()` in Python, where `not` is a keyword (Playwright for Python), and `.not().to_have_url()` in Rust (Playwright for Java). It is generated from the matcher, never a second entry.
  - The settings `plan.frozen`, `extract.mode`, `observation.max_size` and `timeout.ai` follow ADR 0083's casing: `observation.maxSize` in TypeScript, `observation::MAX_SIZE` and `timeout::AI` in Rust.

### Generated and hand-written

- **Generated,** Collie's entries into the Collie client package and Foxhound's into Foxhound's package (ADR 0095):
  - every member a kind expresses, with its option types, result types, doc text and its Experimental and deprecated markers;
  - Python's sync twins and Rust's `blocking` twins;
  - error classes, event types and settings types;
  - the members both Products' Connections share.

  > Note (the owner's decision of 2026-11-12 to do every recommended item), 2026-11-19, [#2517](https://github.com/goodfetch/goodfetch/issues/2517): a round trip, a query or a waiter whose operation's result has exactly one field returns that field's value, and the language's none for an optional one that is absent, so `goto` returns `Response | null` as ADR 0085 says. The rule is the generator's, derived from the catalog with no list of member names; the operation's own function and the wire keep the result type. TypeScript and Rust follow it since #2517, Python with [#2883](https://github.com/goodfetch/goodfetch/issues/2883).

- **Hand-written,** in the Collie client package unless it is a Product's own:
  - ADR 0038's runtime;
  - each Product's Connection type, its `connect()` and scoped helpers;
  - the generic shapes the kinds produce: `Subscription`, iterators, `EventStream`, `Waiter`, Python's `expect_` blocks, Rust's `Call<T>` and `Query<T>`, `Label<P>`;
  - conversions at the boundary: durations, matchers and their regex syntax, schema adapters, JSON value checks;
  - the error base classes and the runtime's own error messages;
  - the testing kits and the raw call;
  - ADR 0089's lending helpers, one per supported library.
- **Every public hand-written member is listed** in its SDK's manifest, with the catalog entry it serves, or "no entry" and a reason.
- **A hand-written member with no entry declares its canonical words in its manifest line,** and each language spells them by its own rule, as it spells a catalog name (S1). ADR 0089's helpers are free functions of the Collie client package, which each Product's package re-exports (ADR 0095), taking the page and then the program's own library object: `lend_to_playwright` and `lend_to_puppeteer` in TypeScript as `lendToPlaywright(page, chromium)` and `lendToPuppeteer(page, puppeteer)`, and `lend_to_playwright` and `lend_to_browser_use` in Python as `lend_to_playwright(page, chromium)` and `lend_to_browser_use(page, BrowserSession)`. A free function keeps each library's name off the page, which has one `lend()` (S2). In Python, `lend_to_playwright` takes a `Page` with Playwright's sync `chromium` or an `AsyncPage` with its async one, typed by overloads, and `lend_to_browser_use` an `AsyncPage`, since Browser Use is async only. The line also declares each language's form (a function, a decorator, a type), so ADR 0100's Agent tool constructor, the words `agent_tool`, is the function `agentTool`, the decorator `@agent_tool` and the type `AgentTool` with `new`.
- **A hand-written member's options follow the catalog's forms** (S1; checked by the Quick suite, below): an options object in TypeScript, keyword-only arguments in Python, and in Rust builder methods named after each option, on the value the member returns or on a declaration value it takes through `impl Into`, never a second member for the same act. ADR 0093's Rust crawl declaration thus reads `crawl.label::<P>(name)` or `crawl.label(LabelDeclaration::<P>::new(name).payload(schema).settings(settings).login(step))`, and its seed `run.add_seed(Seed::new(url, &label, payload))` or `run.add_seed(Seed::sitemap(url, rules))`. `crawl.login(step)` stays: it is already the method named after `new Crawl({ login })`'s option. `blocking::LabelDeclaration` takes a `Fn + Send + Sync + 'static` login (ADR 0081). The same holds for one optional positional argument, `impl Into<Option<…>>` (ADR 0085's `frame_locator(None)` for `frameLocator()`), and for an option of several types, one builder method taking `impl Into` (ADR 0092's `pause().until(…)` for a locator or a URL).
- **The Quick suite fails:**
  - an entry of the Product's catalog that reaches no public member in some SDK;
  - a listed member that lacks its entry's doc text or markers;
  - a manifest line the generator could express, as playwright-python reports an exception it no longer needs (CM A2);
  - a member with no entry not spelled from the words and form its line declares;
  - two members of one receiver listed for the same entry;
  - a hand-written member whose words are missing from the same receiver in another SDK that has that receiver, unless its line says why only one SDK has it, so a Rust-only `label_with` beside `label` fails.

  This closes the drift the research found in hand-written ports (S1, T).

### The raw catalog call (G8)

- **The method.** `call(operation, params?, options?)` exists on the Connection and on every handle, named after ADR 0077's "raw catalog call" and JSON-RPC's own word.
  - It reaches an operation the SDK does not generate, such as one a newer engine minor added, or one of Foxhound's from a Collie program on a Foxhound engine (ADR 0095).
  - `params` and the result are the wire's JSON, in snake_case and never camelCased: TypeScript's and Python's JSON types, `serde_json` values in Rust.
  - Its options are `timeout`, plus `signal` in TypeScript.
  - `params` is positional in TypeScript and Python, though optional, because the raw call keeps the `(method, params)` pair of JSON-RPC and of `CDPSession.send`, ADR 0085's raw CDP way down (S3). Rust has no optional positional argument, so there it is a builder method: `page.call("page.x").params(…).await`.
- **On a value,** `operation` must be in the value's own namespace, and the value fills the parameter of that name.
  - A value's namespace is the singular name of its type in the catalog. A value whose type extends another's, as Foxhound's crawl Run extends a Run, accepts both namespaces.
  - Any other operation fails with a `ProgramError` before it is sent.
  - An operation that acts on no value goes through the Connection, where `params` names every value itself.
- **The main path's protections hold** (G2, G3, G6):
  - a closed value fails with `closed`;
  - deadlines and cancellation apply (ADR 0080);
  - failures raise the same classes, and an unknown code arrives as its group's class (ADR 0079);
  - the SDK never retries, and resends after a reconnect only for read and idempotent classes;
  - the engine validates `params` strictly and enforces every policy;
  - the Run trace records the call, marked raw.
- **Its limits.** It returns the result only: an operation whose use needs events the SDK lacks waits for the SDK that generates them. A parameter that names a second value takes that value's ID, so `call` reaches such an operation only where the program holds one, as a Run's.
- **Semver** covers `call`'s shape, and the catalog's own rules cover what it reaches. No golden scenario uses it (G8).

### Deprecation and Experimental markers (T)

- **The catalog's deprecation of an entry** records `since`, the canonical name of its replacement and the earliest removal. Every marker's text is rendered from it, so there is one text, as ADR 0038 requires.

  > Note (agent decision, on a default, not read by the owner), 2026-11-19, [#2958](https://github.com/goodfetch/goodfetch/issues/2958): `Deprecation` holds `since`, an optional `replacement` in the same catalog section, `earliest_removal`, a later major's `.0`, and the `note`; `Deprecation::message` renders the one text, `` Since <since>, removed no earlier than <earliest removal>: use `<replacement>` instead. <note> ``, which every SDK's marker and warning and the CLI read. A hand-written member warns of its entry's markers once per process as a generated one does: TypeScript's and Python's read the generated registries (`collieOperations`, `EXPERIMENTAL_OPERATIONS`, `DEPRECATED_OPERATIONS`) through `warnEntry` and `warn_entry`, Rust's the catalog itself. A listener's `on` warns the same way of the event it listens to, from the events' registries.

  | Where | Marker |
  |---|---|
  | TypeScript | TSDoc `@deprecated` on the method, property, option field or type; a deprecated argument shape or enum value gets its own overload carrying it. `tsc` reports nothing, so typescript-eslint's and oxlint's `no-deprecated` rules are what flag a use (CM C4) |
  | Python | PEP 702's `@deprecated` (from `typing_extensions` before 3.13) on methods, classes and TypedDict definitions; a deprecated keyword argument or literal value gets an `@overload` carrying it. pyright strict reports a use as an error; mypy needs `--enable-error-code deprecated`, even under `--strict` (CM C1 to C3) |
  | Rust | `#[deprecated(since, note)]` on every item, field, variant and setting constant (CM C5) |
  | Hand-written members | the marker of the entry they serve, checked through the manifest |
  | MCP | a deprecated tool's description opens "Deprecated: use `<tool>`, removed in <major>", and its `inputSchema` root carries the JSON Schema `deprecated: true`; a deprecated argument carries it too. MCP's Tool has no field of its own (CM D1, D6) |
  | CLI | ADR 0030's warning on stderr, and `--help` names the replacement |

- **Where PEP 702 has no marker,** for a field of plain data or a key of a settings `TypedDict` (CM C1, C6), the name keeps ADR 0030's runtime warning and its docstring. Codegen lists each such name in the release notes' deprecations. This is a departure from T's "every deprecated name", recorded as an amendment of ADR 0077.
- **The runtime warning** fires once per process per name, through the host's channel (ADR 0078).
- **Experimental** (ADR 0030):
  - TSDoc's `@experimental`, a Discretionary standard tag (CM C7);
  - a docstring line in Python and in rustdoc;
  - ADR 0030's once-per-process warning.

  No language has a type-level Experimental marker that is not a Cargo feature, which ADR 0038 rules out.

### Other surfaces

Each surface spells the canonical name by one rule (S1), as its leaders do (S3), and keeps its markers (T).

- **MCP tools and Framework adapters' tools** share one name: the canonical name with `_` in place of `.` (`page_goto`, `run_checkpoint`).
  - Only `[a-z0-9_]`, at most 39 characters: OpenAI's and Anthropic's tool names refuse a dot, and OpenAI caps a function name at 64 characters, which a host's 25-character `mcp__goodfetch-foxhound__` prefix shares (CM D1 to D3). Codegen refuses a longer name.
  - Foxhound's carry Collie's under the same names (ADR 0095).
- **CLI commands** read `<executable> <namespace> <member>`, words joined by `-` (`goodfetch-foxhound run checkpoint <run>`).
  - A value-bound operation takes its value's reference as the first positional argument.
  - Flags are `--<parameter>` in kebab-case, as gh, docker and clap's defaults do.
  - `--json` fields stay snake_case (ADR 0038).
  - ADR 0101's `model` parameter is `--model`. Setting keys such as `ai.model` and `ai.providers.<name>` keep their canonical form in the CLI's references and in an MCP call's `settings`, as ADR 0083 has them; the flag syntax that sets a key is the CLI's own design, area 11's.
  - A command group is a singular noun, generated or hand-written (ADR 0069's `daemon start`, `config explain`), as gh's `gh run list` and docker's `docker plugin install` are. A glossary term in the plural keeps its own words (`rules update` for the Clean-up rules).
  - Which operations the CLI offers is area 11's.
- **ADR 0100's Agent** follows these rules: its MCP tool is `page_agent`, and a CLI command for it, where area 11's spec offers one, reads `page agent`. Which MCP profiles include it, and how the CLI and MCP state a Success check inside the task, are area 11's spec, which owns the profiles (ADR 0095) and what the CLI offers; until then, ADR 0100's interim holds, the catalog's `expect` evaluated after the task returns.
- **An entry kept off agent surfaces** (ADR 0011's list, ADR 0041's raising of a Limit, ADR 0029's answer about an uncertain Effect, ADR 0089's `page.lend()`, and [ADR 0092](0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md)'s reading and writing of a Session's cookies and a page's storage) is generated for no MCP tool, Framework adapter, Code mode global or skill. ADR 0092's cookie and storage operations get no CLI command either.
  > Amended by [ADR 0106](0106-a-program-declares-an-effect-as-a-keyed-block-whose-output-the-effect-journal-keeps-judged-by-what-the-engine-sent.md): `effect.open` and `effect.close`, which carry `run.effect`, are kept off them too.
- **Code mode and Script Hooks** keep ADR 0051's TypeScript names.
- **Integrations** use canonical names as identifiers.
- **Reference pages** show the canonical name and each language's spelling.

### Names in errors and in the Run trace (G6)

- **The forms.** `hello` declares the form a Client's user writes, one of six:
  - `typescript`, `python` or `rust` for an SDK, with Code mode as `typescript`;
  - `cli`;
  - `tool` for the MCP server and the Framework adapters;
  - `catalog` for the Console, Integrations and the HTTP listener.
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console. The engine behaviour this ADR describes stands and the Console's part does not; a Client reaches it.
  > Note (agent decision, as built), 2026-10-22, [#817](https://github.com/goodfetch/goodfetch/issues/817): `hello`'s params carry the form as the optional `form` (`typescript`, `python`, `rust`, `cli`, `tool` or `catalog`); the Console names `catalog`, and each other surface names its own with the ticket that renders names in it ([#1272](https://github.com/goodfetch/goodfetch/issues/1272)).
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
  > Note (agent decision, as built), 2026-11-19, [#1272](https://github.com/goodfetch/goodfetch/issues/1272): the TypeScript, Python and Rust SDKs, the linked Rust Client among them, declare their form. A `hello` that declares none takes its surface's, the relayed Client's behind a gateway: `typescript`, `python` or `rust` for an SDK, `typescript` for a Script sandbox worker, `cli` for the CLI, `tool` for the MCP server and `catalog` for every other surface (`Surface::form`).
- **What the engine renders in that form:** every name a message shows (the operation, parameters, setting keys in `limit` and `fix`, events) and targets (ADR 0079). For example, `page.waitForURL` in TypeScript, `page.wait_for_url` in Python, and a setting as `crawl.payload.maxSize`, `crawl.payload.max_size` or `crawl::payload::MAX_SIZE`.
- **One naming crate** serves `xtask codegen` and the engine's renderer, so a message names exactly what the program writes. One renderer per form is tested once, as ADR 0079 argued for targets, and Playwright's server renders locators in its client's `SDKLanguage` (CM A6).
- **Structured fields stay canonical,** so programs match on them across languages: `operation` holds the canonical name, and `limit` keeps ADR 0083's canonical key.
- **The Run trace** records each call's canonical name and its Client's form. The Console shows each call in that form, as Playwright's trace viewer shows actions in the language that recorded them.
  > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.

### The two Products

- **`Connection` is the type both Products' Connections implement,** declared once in the Collie client package (ADR 0095) (S3, T).
  - TypeScript: `interface Connection`, which `Collie` and `Foxhound` implement.
  - Python: base classes `Connection` and `AsyncConnection`, which `Collie`, `Foxhound` and their async twins extend.
  - Rust: `trait Connection` and `blocking::Connection`.
  - Its members are what both Connections share: `engine`, `identities` (the Saved identities of [ADR 0092](0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md): `gf.identities` in TypeScript and Python, `gf.identities()` in Rust, with `list`, `import`, `delete` and `export`, which the keyword rule spells `import_` in Python and `import` in TypeScript and Rust), `call`, `close`, `explain_settings` and Collie's Run start, in canonical words.
  - Every Product's package re-exports it.
- **Rust keeps inherent methods.** `Collie` and `Foxhound` keep their own generated methods of the same names, so a program never imports the trait to call them. The trait serves code written for both Products, which avoids sqlx's "trait not in scope" (CM B7).
- **Why the name.** psycopg's `Connection` and `AsyncConnection`, and sqlx's and diesel's `Connection` traits, use it for the same thing (CM B6, B7).
- **Error classes per Product.**
  - In TypeScript and Python, Collie's code classes live in the client package, and Foxhound's own in Foxhound's package, all extending the client package's group classes. A Collie program that meets a Foxhound code gets its group's class with the code kept (ADR 0079).
- **How `goodfetch-protocol` lays out its types for the Rust crates,** which ADR 0095 hands to this decision so that each SDK crate shows only its own Product's entries:
  - `goodfetch-protocol` keeps every contract type of both Products, as ADR 0038 and ADR 0095 have it, on Collie's side of the crate graph, so one crate still emits the one bundle and catalog file (ADR 0038). Foxhound's own entries sit in its `foxhound` module, so the module is the mark in Rust, and `xtask codegen` emits ADR 0095's mark from where each item is defined, skipping deprecated constants and re-exports, so the two cannot disagree;
  - the client crate, `goodfetch-collie-client` (ADR 0095), re-exports every item outside that module, and Foxhound's SDK crate re-exports the module's items, so Collie's SDK crates show none of Foxhound's entries in their public API, which ADR 0095's check of each crate's public items against its Product's catalog enforces;
  - outside `crates/foxhound/`, only `goodfetch-protocol`, which defines the module, and `xtask` name it. The module holds data, no behaviour, and a check beside ADR 0087's boundary checks fails when another crate outside `crates/foxhound/` names it (amending ADR 0087). Collie's crates meet Foxhound's codes and values only as values of the shared types, and Collie's Size budget (ADR 0087) catches any of the module's code that reaches its executable;
    > Note (owner's decision of 2026-11-10, [#2253](https://github.com/goodfetch/goodfetch/issues/2253) Q3, "every name goes"; as built), 2026-11-12, [#2231](https://github.com/goodfetch/goodfetch/issues/2231): the check is `names-foxhound-module` of `xtask dependency-direction`, which `just lint` runs. It reads every Rust file of each crate outside `crates/foxhound/` but `goodfetch-protocol` and `xtask`, tests, benches and examples included, and fails on a path from `goodfetch_protocol` to `foxhound`. What the engine and the CLI knew by naming the module, or one of Foxhound's names as a string, now arrives another way:
    > - who ends a Run of a kind, how its start commits and which setting says whether its Run trace keeps a Provenance log come with the kind's driver, as a `Driving` value of `Contribution::with_driver`, the way the `Gate` comes with its contribution (ADR 0004); a kind no contribution registers is a Run its program drives;
    > - "fixed for a Run" and "not built yet" are marks of the `Setting` (`fixed_for_run`, `unbuilt`), which the engine reads from the catalog's entries;
    > - the stream of values a sequence's call takes, and the read that bounds a sequence of what a Run committed, are the `Operation`'s (`input`, `bound`), from which the CLI reads `run add-seeds`' values from stdin and ends `run records` without `--follow`; a function such as a bound's `params` is data of an entry, as a Setting's `default` is;
    > - the CLI finds the operations that answer a Run report by the shared `RunReport` type, not by its name in the bundle.
    >
    > What stays, each for its reason:
    > - the generated `foxhound_names` (ADR 0087, "How an engine knows"), which the engine's Connection and the CLI's `other_product` read to refuse with `not_in_product` what only Foxhound carries, and which the test tools (`goodfetch-test-client`, `goodfetch-conformance`) read to send Foxhound's calls without its types: the table exists for that refusal, holds names and no type, and is no part of the module;
    > - the CLI's paging (`after`, `next`, `ended` beside one list of items), read from each operation's schemas: a shape any operation of the shared catalog may take, and no name of Foxhound's;
    > - `RunKind`, a shared type: no code outside `crates/foxhound/` matches on its `crawl` value any more, which this check cannot see.
  - a type both Products share whose values Foxhound extends (Error codes, Usage kinds, Limit scopes, Notification events, Plugin kinds, Hook stages) is an open value type: a newtype holding its wire string inline. Each Collie value is an associated constant of the type (`ErrorCode::TARGET_NOT_UNIQUE`), which every re-export of the type carries. Each Foxhound value is a free constant of the same type in the `foxhound` module, re-exported by Foxhound's crate (`goodfetch_foxhound::error_code::PAYLOAD_INVALID`);
  - such a value has one in-memory form whoever makes it: inline, at most 63 bytes, which codegen checks on every value of the catalog. So a value read from the wire equals its constant with `==` and in a `match`, whichever crate defines the constant, as `http::Method`'s and `http::StatusCode`'s constants are matched. A value the program does not know stays a value, with `as_str()` (G3);
  - an entry that moves from Foxhound to Collie in a minor (ADR 0095) keeps every name Foxhound's crate showed for it: a value's free constant stays as a `#[deprecated]` constant equal to the new associated constant, through ADR 0030's deprecation window, and a type stays re-exported at its old path. A Foxhound program that matches the old constant still compiles, with a warning, so the move stays additive, as ADR 0030's minors require and ADR 0088 expects of adding Paid fetches to Collie later (T);
  - an Error code's typed details are read with `details::<D>()`, where `D` is the code's details type and Foxhound's types live in the `foxhound` module, never through an enum that lists every code's details;
  - this replaces ADR 0079's Rust `ErrorCode` and `ErrorDetails` enums. One enum listing Foxhound's codes would put them in Collie's SDK crates' public API, an enum cannot be extended by another crate, and a second Foxhound enum would give Foxhound programs two code types to match (S2).

## Considered Options

- **Per-entry, per-language annotations in the catalog,** as Playwright's `* langs:` lines: rejected. Every catalog author would need three languages, and names would drift per language, which ADR 0079 already refused.
- **Hand-writing the idiomatic layer over a raw generated one,** as Playwright's clients: rejected. It is how ports drift (P43).
- **The same shape in every language,** as Stagehand v4's: rejected by ADR 0077.
- **Plural namespaces** (`runs.start` and `schedules.edit` in ADRs 0036, 0038 and 0041 and area 3's spec): rejected. A value-bound operation acts on one value, and `run.checkpoint` then reads alike on every surface.
- **Namespace objects on the Connection** (`collie.run.checkpoint(id)`), as Stainless's resources: rejected. Programs would pass IDs of values they already hold (G2).
- **A flat Connection of verb-noun methods,** as Azure's clients: rejected for the same reason.
- **Initialisms as plain words everywhere** (`waitForUrl`, `baseUrl`): rejected. It breaks Playwright's names, which ADR 0080 and ADR 0083's option names follow (S3).
- **Playwright's names in its casing and Goodfetch's own as words,** where the sketch had drifted (`waitForURL` beside `UrlMatcher`): rejected. Two rules inside one language (S1).
- **Banning every reserved word from canonical names:** rejected. `route.continue` would lose Playwright's name (S3).
- **Trailing underscores in Rust,** as playwright-rs: rejected. rustc's own help leads to `r#`, and Rust's generated SDKs use it.
- **Per-language renames,** as Java's `navigate`: rejected by S1.
- **Aliases kept for familiarity,** with or without a warning: rejected by S2 and T.
- **A `readonly` property for a fact fixed at creation in TypeScript,** with methods only for reads that change: rejected. The page API keeps Playwright's methods for fixed facts (`request.url()`), so TypeScript would show two forms for one kind (S1). **Properties for every read**, as fetch's `Response`: rejected. Playwright's page API is the leader there (S3).
- **No raw call,** a newer operation waiting for an SDK upgrade: rejected by G8.
- **A raw call on the Connection only,** with each value exposing a wire ID for the program to place: rejected. It adds a member to every value and leaves the placement to the program.
- **A raw call that also surfaces events:** rejected. Unknown events would need a second, untyped event API.
- **Pydantic's `Field(deprecated=…)` as Python's marker for a field:** rejected as the only marker. Its docs promise a runtime warning and the JSON Schema keyword only, and mypy does not flag it (CM C6).
- **A Tool-level `deprecated` field in MCP:** not available (CM D1). The description and JSON Schema's annotation are what hosts and models read.
- **Dotted MCP tool names** (`page.goto`), which MCP allows: rejected. OpenAI and Anthropic refuse them as tool names (CM D2).
- **One rendering form per SDK only,** leaving the CLI's and MCP's to their own designs: rejected. A model calling `page_goto` should read the names it called.
- **`CollieConnection`, `EngineConnection`, `Goodfetch` or `Client` for the shared type.**
  - `CollieConnection` names a Product on a type Foxhound programs hold, with a long async twin.
  - `EngineConnection` is longer, and ADR 0078 rejected "engine" for the Connection, since closing it never stops a Daemon.
  - ADR 0090 rejected `Goodfetch`, which names nothing installed, and ADR 0078 rejected `Client`, the glossary's word for the program.
- **The shared members on the Rust trait only:** rejected. Every program would import the trait (rustc E0599).
- **One Rust `ErrorCode` enum listing both Products' codes:** rejected. It puts Foxhound's names in Collie's SDK crates' public API (ADR 0095). **Foxhound's variants behind a Cargo feature:** rejected. A Cargo feature exists only for heavy optional dependencies and test utilities (ADR 0038), and types would change with features, as Spider's do (ADR 0083). **A second Rust enum for Foxhound's codes:** rejected by S2.
- **`_with` variants in Rust** (`label_with`, `login_for`), as std's `Vec::with_capacity`: rejected. They are a second name for one act (S1), while std's `with_` names are secondary constructors of a type; Rust's options are builder methods everywhere else in the SDK, as `Seed` already has them.
- **Foxhound's contract types in a crate under `crates/foxhound/`,** which ADR 0087's "never names a Foxhound type" read literally would give: rejected. Two crates would define the contracts and feed the one bundle and catalog file that ADR 0038 has one crate emit, and ADR 0095 keeps every entry's mark beside the catalog in `goodfetch-protocol`. The module keeps ADR 0087's purpose, no Collie code naming Foxhound's, with a check.

## Consequences

- **Amends ADR 0038.**
  - Canonical names are snake_case words, and operations are `<namespace>.<member>` with singular namespaces.
  - Each language's spelling, initialisms included, comes from one rule per language.
  - The catalog declares kinds, and generators apply one form per kind.
  - Hand-written members are listed in a manifest the Quick suite checks.
- **Amends ADR 0079.**
  - `operation` holds the canonical name, rendered in the caller's spelling in the message.
  - Class names follow this ADR's initialism rule (`CDPCommandRefusedError`).
  - The language `hello` carries is one of six forms, and names besides targets are rendered in it.
  - Rust's `ErrorCode` is an open value type with constants, and Rust's error details are read by type, no longer through enums.
- **Amends ADR 0085.** Its raw CDP way down is spelled `page.newCDPSession()` returning a `CDPSession` in TypeScript, `page.new_cdp_session()` returning a `CDPSession` in Python, and `page.new_cdp_session()` returning a `CdpSession` in Rust; Rust's route answer is `r#continue()`. Rust's selector-less frame locator is `frame_locator(None)`, with no `any_frame()`. Notes sit under its raw CDP item and its forms per language.
- **Amends ADR 0087.** Foxhound's contract types live in `goodfetch-protocol`'s `foxhound` module, which no crate outside `crates/foxhound/` but `goodfetch-protocol` and `xtask` names, a check beside its boundary checks. So `goodfetch-protocol`, a crate both Products link, holds more of Foxhound's than its names, in a module Collie's crates never name. Notes sit under its "never names a Foxhound type" item and its "How an engine knows" item.
- **Amends ADR 0083.** "Camelcases each segment" applies the initialism rule (`baseURL`).
- **Amends ADR 0030.**
  - Each language's deprecation marker and its gaps are as above.
  - MCP's "`deprecated` in the tool schema" is the description's first sentence and JSON Schema's `deprecated`.
  - Experimental's per-language marks are as above.
- **Amends ADR 0077.** T's "every deprecated name carries the language's type-level marker" holds where the language has one; a Python plain-data field or settings `TypedDict` key keeps the runtime warning and is listed in the release notes.
- **Amends ADR 0069.** Its completeness tests also check the hand-written manifest, its markers, its unused lines, the spelling of members with no entry, duplicate members for one entry and members one SDK alone has.
- **Amends ADR 0092.** Rust's `page.pause()` has one `.until(..)` taking `impl Into<PauseUntil>`, with no `.until_url(..)`. A note sits under its "What it takes" item.
- **Amends ADR 0093.** Rust's Label declaration is one `crawl.label` taking a name or a `LabelDeclaration` with `payload`, `settings` and `login` builder methods, so `label_with` and `login_for` are gone, and Rust's `run.add` is `run.add_seed`, taking `impl Into<Seed>`. Notes sit under its login step, its Rust declaration and its `run.addSeed` items.
- **Amends ADR 0103.** Rust's Dataset declaration is one `crawl.dataset`, taking a name or a `DatasetDeclaration` with `schema` and `key` builder methods, so `dataset_with` is gone. A note sits under its declaration item.
- **Amends ADR 0095.** Its shared connection type is `Connection`; `goodfetch-protocol` lays out Foxhound's types in its `foxhound` module, from which `xtask codegen` emits the mark; and the Rust SDKs generate their members and `blocking` twins, reusing `goodfetch-protocol`'s types. Notes sit under its shared members item and its "Foxhound's entries stay out of Collie's crates' public API" item.
- **Amends ADR 0097.** Its generated reference pages show each entry's canonical name and its spelling in each language, which area 1's generator builds. A note sits under its reference pages item.
- **Amends ADR 0101.** Its `ai.*` keys and `model` parameter are spelled by this ADR's rules on the CLI and MCP; how a CLI flag sets a key stays the CLI's own design (ADR 0083), area 11's. A note sits under its hand-off to this ticket.
- **Amends ADR 0100.** Its Agent's MCP tool and any CLI command are named by this ADR's rules, and its profiles and in-task Success check on the CLI and MCP go to area 11's spec. Notes sit under its "Unchanged from ADR 0011" item and its hand-off to this ticket.
- **Reading rules for earlier ADRs and specs:**
  - an operation namespace written in the plural reads singular: `runs.<x>` is `run.<x>` (ADRs 0031, 0036, 0038, 0041, 0054, 0055, 0068, 0072 and area 3's spec) and `schedules.<x>` is `schedule.<x>` (ADRs 0036, 0041 and ADR 0037's area order); setting keys keep their areas (ADR 0083);
  - a CLI command group written in the plural reads singular: `runs reindex` is `run reindex`, `plugins add` is `plugin add`, `certs accept` is `cert accept`, `browsers install` is `browser install`, `schedules apply` is `schedule apply`, `skills install` is `skill install`, `sinks list` is `sink list`; `rules update` keeps the Clean-up rules' own word;
  - a name written in one language's spelling (`JsonValue`, `CdpSession`, `AiWorkResult`, `newCdpSession()`, a TypeScript read written as a property) is spelled in each SDK by this ADR's rule.
- **`GLOSSARY.md`** gains **Connection**, and its Limit and Usage ledger entries say "a Connection".
- **`STYLE.md`'s** Names section points to this ADR, and its SDK section adds the manifest.
- **`TESTING.md`** gains catalog-mapping conformance cases.
- **ADR 0037's matrix.** SDK-02, SDK-05, SDK-06, AI-06 and AI-07 cite this ADR. Areas 1, 3, 11 and 12 list it.
- **Areas.**
  - Area 1 (Repository, CI and release) shows each reference page's canonical name and per-language spellings in ADR 0097's generator.
  - Area 3 (Engine runtime) builds the canonical names and kinds in `goodfetch-protocol`, its `foxhound` module and the check that no crate outside `crates/foxhound/` but `goodfetch-protocol` and `xtask` names it, the naming crate, the six forms of `hello` and their renderers, and the Run trace's forms.
  - Area 11 (Client surfaces) decides which MCP profiles include ADR 0100's Agent and how the CLI and MCP state its Success check inside the task, and builds the generators, the manifests and their checks, `call`, the markers, `Connection`, and the CLI's and MCP's names.
  - Area 12 (Console) shows each call in its Client's form.
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
- **Other tickets.**
  - [API sketch: type-checked stubs and golden scenarios](https://github.com/goodfetch/goodfetch/issues/483) applies the remaining renames: initialisms (`JSONValue`, `URLMatcher`, `PageAPI`, `AIWorkResult` and the error classes), TypeScript reads on handles as methods (`run.id()`, `run.startedAs()`), Rust's open `ErrorCode` and `details::<D>()`, `call` on every handle, ADR 0089's lending helpers by the names above, and ADR 0090's package and import names in place of the sketch's placeholder `goodfetch`.
  - [The Run in code](https://github.com/goodfetch/goodfetch/issues/478) (ADR 0102) spells its TypeScript reads as methods (`run.id()`, `run.startedAs()`).
  - [Agent-writability check](https://github.com/goodfetch/goodfetch/issues/484) runs pyright strict, mypy with `--enable-error-code deprecated` and oxlint's `no-deprecated`.
- **The `prototype/sdk-api` sketch** holds this ADR's signatures in its `catalog` files for each language ([`b9eaed1`](https://github.com/goodfetch/goodfetch/commit/b9eaed1), [`251bda0`](https://github.com/goodfetch/goodfetch/commit/251bda0), [`042feed`](https://github.com/goodfetch/goodfetch/commit/042feed), [`98b3482`](https://github.com/goodfetch/goodfetch/commit/98b3482), [`943382c`](https://github.com/goodfetch/goodfetch/commit/943382c), [`e0d11d2`](https://github.com/goodfetch/goodfetch/commit/e0d11d2), [`5258b15`](https://github.com/goodfetch/goodfetch/commit/5258b15)). The pain coverage matrix closes P07, P33, P43 and K06 as API.
