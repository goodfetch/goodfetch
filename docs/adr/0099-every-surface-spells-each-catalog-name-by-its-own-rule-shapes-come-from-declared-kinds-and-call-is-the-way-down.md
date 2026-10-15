# Every surface spells each catalog name by its own rule, shapes come from declared kinds, and `call` is the way down

One Operation catalog generates every surface (ADRs 0011, 0038), and this decides how an entry becomes a TypeScript, Python or Rust member, a CLI command, an MCP tool and the names an error shows.

**The decision in brief.**
- Each entry has one canonical name made of snake_case words, which each language spells by one rule, initialisms included.
- The catalog declares what an entry is, never how a language shows it, so each language's generator applies one form per kind, with no per-entry override and no alias.
- Hand-written members are listed against the entries they serve, so a missing or stale one fails the build.
- A raw `call` is the Client layer's way down.
- Deprecations carry each language's type-level marker.
- The engine names everything in the caller's own spelling.
- Both Products' connections implement one `Connection` type, and each Rust crate shows only its own Product's entries.

**Evidence.**
- **Playwright** keeps per-language aliases and language-only entries in 461 `* langs:` lines, and playwright-python a table of reasoned exceptions ([browser drivers §4.3](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/browser-drivers.md)). So its languages differ entry by entry: `AbortSignal` is JavaScript-only, and Java renames `goto` and `continue`.
- **Smithy** turns language-neutral traits into each language's idiom.
- **The languages disagree** on initialisms (`HTTPServerError` in PEP 8, `Uuid` in Rust's C-CASE, `waitForURL` in Playwright) and on keywords (PEP 8's trailing underscore, rustc's `r#`).
- **`tsc` never reports a deprecation**, mypy only on request, and MCP has no deprecated field.

These were checked against pinned sources on 2026-10-15 in the [catalog mapping research](https://github.com/goodfetch/automan/blob/research/sdk-catalog-mapping/rnd/research/2026-10-15-sdk-catalog-mapping.md) (CM below), beside the [SDK design guidelines research](https://github.com/goodfetch/automan/blob/research/sdk-design-guidelines/rnd/research/2026-10-14-sdk-design-guidelines.md) (lessons 13 and 30 to 35).

Decided in [Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461), by the agent on the owner's delegation of 2026-10-14 ([rounds 1 to 3](https://github.com/goodfetch/automan/issues/471#issuecomment-6181920227), [grilling record](https://github.com/goodfetch/automan/issues/471#issuecomment-6181926808)). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md).

## Decision

### Names

- **One canonical name per entry.**
  - Every operation, parameter, result field, event, Error code and setting key has one name in `goodfetch-protocol`: snake_case ASCII words, each `[a-z][a-z0-9]*`. Word boundaries are thus explicit, as ADR 0038's wire and ADR 0083's keys already are.
  - Every surface derives its spelling from these words by its own rule.
  - No entry has a per-language override (S1). When a rule gives a bad result, the canonical name changes for every surface, as ADR 0079 does for a class name that would shadow a built-in.
- **An operation is `<namespace>.<member>`.**
  - The namespace is the value the operation acts on or concerns, singular: `page.goto`, `locator.click`, `run.checkpoint`, `run.start`. Setting keys keep ADR 0083's areas. An operation is called on one value, and one noun per value then reads the same in the SDK, the CLI (`run checkpoint <run>`, as `gh run view`), MCP and errors.
  - An operation that acts on a value takes it as its first parameter, named after the namespace.
- **The value an operation acts on owns it** (G2, S3). An operation whose first parameter is a value the SDK hands out is a method of that value, under its canonical member, and the SDK fills the parameter: `run.checkpoint()`.
- **An operation that acts on no value is a member of the connection.** Its catalog entry declares the member's name, chosen by the ticket or spec that owns it (`run.start` is `start`).
- **One name means one operation on one value, in every language.** Codegen refuses two entries that would give one receiver the same member, across a Product's whole catalog, since Rust has no overloading (S1). One operation may still take alternative argument shapes: TypeScript overloads, Python's `@overload`, a Rust `impl Into<…>` parameter.
- **Casing.**

  | | TypeScript | Python | Rust |
  |---|---|---|---|
  | Members, parameters, fields | camelCase: `waitForURL`, `baseURL`, `url()` | snake_case: `wait_for_url` | snake_case: `wait_for_url` |
  | Types and classes | PascalCase: `CDPSession`, `JSONValue`, `URLMatcher` | PascalCase: `CDPSession`, `AsyncCDPSession`, `CDPCommandRefusedError` | UpperCamelCase: `CdpSession`, `JsonValue` |
  | Setting keys (ADR 0083) | nested camelCase segments | nested snake_case segments | modules and an UPPER_SNAKE constant |

- **Initialisms.**
  - They come from a closed list kept in `xtask codegen`: `ai`, `api`, `cdp`, `css`, `dom`, `html`, `http`, `https`, `json`, `mcp`, `pdf`, `uri`, `url`, `xml`.
  - In TypeScript names and Python class names an initialism is in capitals, except at the start of a camelCase name, where it is lowercase. In Rust it is one word.
  - This follows each language's own leaders: Playwright, the web platform, Stainless (`APIError`, `baseURL`) and `@types/json-schema` (`JSONSchema7`) in TypeScript; PEP 8, the standard library, httpx and Playwright for Python in Python; C-CASE in Rust (CM A3, A4, B1, B2, B6, B8).
  - An abbreviation that is a word stays one: `id` gives `runId`, `timezoneId` and `getByTestId`, as in Playwright.
  - A word joins the list before any released name uses it, since adding it later renames that name.
- **Event names** passed to `on()` are the event's words joined in lowercase in TypeScript and Python (`requestfinished`, `pageerror`, `websocket`), as Playwright and the DOM spell events (CM A4). The wire keeps snake_case, and Rust's marker types are UpperCamelCase (`events::RequestFinished`), as ADR 0081's sketch has them.
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

  - A read is named for what it returns, never with `get_`, except names kept from Playwright (`getAttribute`, `getBy*`).
  - `set_` changes state the value itself holds, as Playwright's gestures (`setContent`, `setInputFiles`) and a Run's progress message do, never a setting (ADR 0083).
  - Other verbs, such as starting or resuming a Run, are their owning ticket's.

### Shapes

- **The catalog declares each entry's kind, and each language has one form per kind** (S1, S3, T). Most rows restate ADRs 0078 to 0083, so that codegen applies them without exceptions:

  | Kind | TypeScript | Python | Rust |
  |---|---|---|---|
  | Round trip | `async` method | `async def` in an `Async…` class, a plain method in its sync twin | method returning a `Call<T>` builder, awaited; `send()` in `blocking` |
  | A read the SDK answers itself, on a handle | method, as all of Playwright's are (`page.url()`, `response.ok()`, `run.id()`, `connection.engine()`) | a read with no arguments is a property, except a name starting `is_` or `as_`, which is a method: Playwright for Python's generator rule (`response.ok`, `page.url`, `run.id`, `page.is_closed()`, CM A2) | method (C-GETTER) |
  | Builder without I/O (`getByRole`, `filter`) | sync method | sync method | method |
  | Sub-object without I/O (`keyboard`, `request`, `identities`) | `readonly` property | property | method |
  | Waiter (ADR 0080) | `waitForX(…)` | `wait_for_x(…)`, and an `expect_x` block for an event the program's own action triggers | `wait_for_x(…)` returning a `Waiter` with `during(action)` |
  | Side-activity event (ADR 0081) | `on('x', listener)` returning a `Subscription` | the same | `events(events::X)` returning an `EventStream` |
  | Sequence (ADR 0081) | `AsyncIterable` | async iterator; iterator in the sync API | `EventStream`; `Iterator` in `blocking` |
  | Opener | the value, with `await using` | the value, with `async with` or `with` | the value, with `close()` |
  | Cancellable (ADR 0080) | `signal` option | task cancellation | dropping the future |
  | Duration (ADR 0080) | `number` of milliseconds | `timedelta` | `Duration` |
  | Point in time (Sessions, logins, Human pause and secrets in code) | `Date` | time-zone-aware `datetime` in UTC | `SystemTime` |

- **One form for reads in TypeScript.** A read on a handle is a method whether it may change (`page.url()`) or is fixed at creation (`request.url()`, `run.id()`): one form in the language (S1), Playwright's (S3), which ADR 0085 keeps for the page API. A property would have suited a fixed fact better for T, since a method used as a value can pass unseen in a template string, but T ranks below both.
- **Plain data** (results, error fields, event data that is not a page-API class):
  - TypeScript: `readonly` properties;
  - Python: attributes;
  - Rust: public fields of `#[non_exhaustive]` structs, with `goodfetch::Error` staying opaque (ADR 0079).
  - A field only Foxhound fills belongs to a Foxhound type, never to a type both Products share (as `CrawlRun` beside `Run`), since a Rust struct cannot hide a field from one crate.
- **Parameters.**
  - Required parameters are positional, in the catalog's order, and a boolean is never positional.
  - Optional parameters go in a trailing options object in TypeScript, as keyword-only arguments in Python, and as builder methods named after the parameter in Rust.
  - Three states follow ADR 0083.
- **Typed defaults.** Signatures carry no default value, since defaults are settings the engine resolves (ADR 0083). Doc comments state each default in the language's own type: `30_000`, `timedelta(seconds=30)` and `Duration::from_secs(30)` for 30 s.
- **Twins.**
  - Python: every class with a round trip has an `Async<Name>` twin in the same module with the same members, as ADR 0090's `Collie` and `AsyncCollie`, httpx and psycopg have them. Plain data, errors, settings types and `Label` serve both.
  - Rust: `blocking` repeats the async API under the same type names (ADR 0081).
  - TypeScript: async only.

### Generated and hand-written

- **Generated,** Collie's entries into the Collie client package and Foxhound's into Foxhound's package (ADR 0095):
  - every member a kind expresses, with its option types, result types, doc text and its Experimental and deprecated markers;
  - Python's sync twins and Rust's `blocking` twins;
  - error classes, event types and settings types;
  - the members both Products' connections share.
- **Hand-written,** in the Collie client package unless it is a Product's own:
  - ADR 0038's runtime;
  - each Product's connection type, its `connect()` and scoped helpers;
  - the generic shapes the kinds produce: `Subscription`, iterators, `EventStream`, `Waiter`, Python's `expect_` blocks, Rust's `Call<T>`, `Label<P>`;
  - conversions at the boundary: durations, matchers and their regex syntax, schema adapters, JSON value checks;
  - the error base classes and the runtime's own error messages;
  - the testing kits and the raw call.
- **Every public hand-written member is listed** in its SDK's manifest, with the catalog entry it serves, or "no entry" and a reason.
- **The Quick suite fails:**
  - an entry of the Product's catalog that reaches no public member in some SDK;
  - a listed member that lacks its entry's doc text or markers;
  - a manifest line the generator could express, as playwright-python reports an exception it no longer needs (CM A2).

  This closes the drift the research found in hand-written ports (S1, T).

### The raw catalog call (G8)

- **The method.** `call(operation, params?, options?)` exists on the connection and on every handle, named after ADR 0077's "raw catalog call" and JSON-RPC's own word.
  - It reaches an operation the SDK does not generate, such as one a newer engine minor added, or one of Foxhound's from a Collie program on a Foxhound engine (ADR 0095).
  - `params` and the result are the wire's JSON, in snake_case and never camelCased: TypeScript's and Python's JSON types, `serde_json` values in Rust.
  - Its options are `timeout`, plus `signal` in TypeScript. `params` is optional, so in Rust it is a builder method: `page.call("page.x").params(…).await`.
- **On a value,** `operation` must be in the value's own namespace, and the value fills the parameter of that name. Any other operation fails with a `ProgramError` before it is sent. On the connection, `params` names every value itself.
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

- **MCP tools and Framework adapters' tools** share one name: the canonical name with `_` in place of `.` (`page_goto`, `run_checkpoint`).
  - Only `[a-z0-9_]`, at most 39 characters: OpenAI's and Anthropic's tool names refuse a dot, and OpenAI caps a function name at 64 characters, which a host's 25-character `mcp__goodfetch-foxhound__` prefix shares (CM D1 to D3). Codegen refuses a longer name.
  - Foxhound's carry Collie's under the same names (ADR 0095).
- **CLI commands** read `<executable> <namespace> <member>`, words joined by `-` (`goodfetch-foxhound run checkpoint <run>`).
  - A value-bound operation takes its value's reference as the first positional argument.
  - Flags are `--<parameter>` in kebab-case, as gh, docker and clap's defaults do.
  - `--json` fields stay snake_case (ADR 0038).
  - A command group is a singular noun, generated or hand-written (ADR 0069's `daemon start`, `config explain`), as gh's `gh run list` and docker's `docker plugin install` are. A glossary term in the plural keeps its own words (`rules update` for the Clean-up rules).
  - Which operations the CLI offers is area 11's.
- **An entry kept off agent surfaces** (ADR 0011's list, ADR 0041's raising of a Limit, ADR 0029's answer about an uncertain Effect, and the cookie operations Sessions, logins, Human pause and secrets in code offers to no Client acting for a model) is generated for no MCP tool, Framework adapter, Code mode global or skill.
- **Code mode and Script Hooks** keep ADR 0051's TypeScript names.
- **Integrations** use canonical names as identifiers.
- **Reference pages** show the canonical name and each language's spelling.

### Names in errors and in the Run trace (G6)

- **The forms.** `hello` declares the form a Client's user writes, one of six:
  - `typescript`, `python` or `rust` for an SDK, with Code mode as `typescript`;
  - `cli`;
  - `tool` for the MCP server and the Framework adapters;
  - `catalog` for the Console, Integrations and the HTTP listener.
- **What the engine renders in that form:** every name a message shows (the operation, parameters, setting keys in `limit` and `fix`, events) and targets (ADR 0079). For example, `page.waitForURL` in TypeScript, `page.wait_for_url` in Python, and a setting as `crawl.payload.maxSize`, `crawl.payload.max_size` or `crawl::payload::MAX_SIZE`.
- **One naming crate** serves `xtask codegen` and the engine's renderer, so a message names exactly what the program writes. One renderer per form is tested once, as ADR 0079 argued for targets, and Playwright's server renders locators in its client's `SDKLanguage` (CM A6).
- **Structured fields stay canonical,** so programs match on them across languages: `operation` holds the canonical name, and `limit` keeps ADR 0083's canonical key.
- **The Run trace** records each call's canonical name and its Client's form. The Console shows each call in that form, as Playwright's trace viewer shows actions in the language that recorded them.

### The two Products

- **`Connection` is the type both Products' connections implement,** declared once in the Collie client package (ADR 0095) (S3, T).
  - TypeScript: `interface Connection`, which `Collie` and `Foxhound` implement.
  - Python: base classes `Connection` and `AsyncConnection`, which `Collie`, `Foxhound` and their async twins extend.
  - Rust: `trait Connection` and `blocking::Connection`.
  - Its members are what both connections share: `engine`, `identities` (the Saved identities of [Sessions, logins, Human pause and secrets in code](https://github.com/goodfetch/automan/issues/476)), `call`, `close`, `explainSettings` and Collie's Run start.
  - Every Product's package re-exports it.
- **Rust keeps inherent methods.** `Collie` and `Foxhound` keep their own generated methods of the same names, so a program never imports the trait to call them. The trait serves code written for both Products, which avoids sqlx's "trait not in scope" (CM B7).
- **Why the name.** psycopg's `Connection` and `AsyncConnection`, and sqlx's and diesel's `Connection` traits, use it for the same thing (CM B6, B7).
- **Error classes per Product.**
  - In TypeScript and Python, Collie's code classes live in the client package, and Foxhound's own in Foxhound's package, all extending the client package's group classes. A Collie program that meets a Foxhound code gets its group's class with the code kept (ADR 0079).
- **How `goodfetch-protocol` lays out its types for the Rust crates,** which ADR 0095 hands to this decision so that each SDK crate shows only its own Product's entries:
  - a type wholly one Product's is re-exported only by that Product's crate, and settings constants, events and values stay with their Product's crate;
  - a type both Products share whose values Foxhound extends (Error codes, Usage kinds, Limit scopes, Notification events) is an open value type: a newtype over its wire string, with an associated constant for each Collie value in the client crate (`ErrorCode::TARGET_NOT_UNIQUE`) and a constant of the same type in Foxhound's crate for each Foxhound value (`goodfetch_foxhound::error_code::PAYLOAD_INVALID`);
  - a program compares such a value with `==` or matches it against the constants, as `http::StatusCode` and `http::Method` are matched, and a value it does not know stays a value, with `as_str()` (G3);
  - this replaces ADR 0079's Rust `ErrorCode` enum. One enum listing Foxhound's codes would put them in Collie's crate, an enum cannot be extended by another crate, and a second Foxhound enum would give Foxhound programs two code types to match (S2).

## Considered Options

- **Per-entry, per-language annotations in the catalog,** as Playwright's `* langs:` lines: rejected. Every catalog author would need three languages, and names would drift per language, which ADR 0079 already refused.
- **Hand-writing the idiomatic layer over a raw generated one,** as Playwright's clients: rejected. It is how ports drift (P43).
- **The same shape in every language,** as Stagehand v4's: rejected by ADR 0077.
- **Plural namespaces** (`runs.start` and `schedules.edit` in ADRs 0036, 0038 and 0041 and area 3's spec): rejected. A value-bound operation acts on one value, and `run.checkpoint` then reads alike on every surface.
- **Namespace objects on the connection** (`collie.run.checkpoint(id)`), as Stainless's resources: rejected. Programs would pass IDs of values they already hold (G2).
- **A flat connection of verb-noun methods,** as Azure's clients: rejected for the same reason.
- **Initialisms as plain words everywhere** (`waitForUrl`, `baseUrl`): rejected. It breaks Playwright's names, which ADR 0080 and ADR 0083's option names follow (S3).
- **Playwright's names in its casing and Goodfetch's own as words,** where the sketch had drifted (`waitForURL` beside `UrlMatcher`): rejected. Two rules inside one language (S1).
- **Banning every reserved word from canonical names:** rejected. `route.continue` would lose Playwright's name (S3).
- **Trailing underscores in Rust,** as playwright-rs: rejected. rustc's own help leads to `r#`, and Rust's generated SDKs use it.
- **Per-language renames,** as Java's `navigate`: rejected by S1.
- **Aliases kept for familiarity,** with or without a warning: rejected by S2 and T.
- **A `readonly` property for a fact fixed at creation in TypeScript,** with methods only for reads that change: rejected. The page API keeps Playwright's methods for fixed facts (`request.url()`), so TypeScript would show two forms for one kind (S1). **Properties for every read**, as fetch's `Response`: rejected. Playwright's page API is the leader there (S3).
- **No raw call,** a newer operation waiting for an SDK upgrade: rejected by G8.
- **A raw call on the connection only,** with each value exposing a wire ID for the program to place: rejected. It adds a member to every value and leaves the placement to the program.
- **A raw call that also surfaces events:** rejected. Unknown events would need a second, untyped event API.
- **Pydantic's `Field(deprecated=…)` as Python's marker for a field:** rejected as the only marker. Its docs promise a runtime warning and the JSON Schema keyword only, and mypy does not flag it (CM C6).
- **A Tool-level `deprecated` field in MCP:** not available (CM D1). The description and JSON Schema's annotation are what hosts and models read.
- **Dotted MCP tool names** (`page.goto`), which MCP allows: rejected. OpenAI and Anthropic refuse them as tool names (CM D2).
- **One rendering form per SDK only,** leaving the CLI's and MCP's to their own designs: rejected. A model calling `page_goto` should read the names it called.
- **`CollieConnection`, `EngineConnection`, `Goodfetch` or `Client` for the shared type.**
  - `CollieConnection` names a Product on a type Foxhound programs hold, with a long async twin.
  - `EngineConnection` is longer, and ADR 0078 rejected "engine" for the connection, since closing it never stops a Daemon.
  - ADR 0090 rejected `Goodfetch`, which names nothing installed, and ADR 0078 rejected `Client`, the glossary's word for the program.
- **The shared members on the Rust trait only:** rejected. Every program would import the trait (rustc E0599).
- **One Rust `ErrorCode` enum listing both Products' codes:** rejected. It puts Foxhound's names in Collie's crate (ADR 0095). **Foxhound's variants behind a Cargo feature:** rejected. A Cargo feature exists only for heavy optional dependencies and test utilities (ADR 0038), and types would change with features, as Spider's do (ADR 0083). **A second Rust enum for Foxhound's codes:** rejected by S2.

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
  - Rust's `ErrorCode` is an open value type with constants, no longer an enum.
- **Amends ADR 0083.** "Camelcases each segment" applies the initialism rule (`baseURL`).
- **Amends ADR 0030.**
  - Each language's deprecation marker and its gaps are as above.
  - MCP's "`deprecated` in the tool schema" is the description's first sentence and JSON Schema's `deprecated`.
  - Experimental's per-language marks are as above.
- **Amends ADR 0077.** T's "every deprecated name carries the language's type-level marker" holds where the language has one; a Python plain-data field or settings `TypedDict` key keeps the runtime warning and is listed in the release notes.
- **Amends ADR 0069.** Its completeness tests also check the hand-written manifest, its markers and its unused lines.
- **Reading rules for earlier ADRs and specs:**
  - an operation namespace written in the plural reads singular: `runs.<x>` is `run.<x>` (ADRs 0031, 0036, 0038, 0041, 0054, 0055, 0068, 0072 and area 3's spec) and `schedules.<x>` is `schedule.<x>` (ADRs 0036, 0041 and ADR 0037's area order); setting keys keep their areas (ADR 0083);
  - a CLI command group written in the plural reads singular: `goodfetch runs reindex` is `run reindex`, `plugins add` is `plugin add`, `certs accept` is `cert accept`, `browsers install` is `browser install`, `schedules apply` is `schedule apply`, `skills install` is `skill install`, `sinks list` is `sink list`; `rules update` keeps the Clean-up rules' own word;
  - a name written in one language's spelling (`JsonValue`, `CdpSession`, `AiWorkResult`, `newCdpSession()`, a TypeScript read written as a property) is spelled in each SDK by this ADR's rule.
- **`GLOSSARY.md`** gains **Connection**, and its Limit and Usage ledger entries say "a Connection".
- **`STYLE.md`'s** Names section points to this ADR, and its SDK section adds the manifest.
- **`TESTING.md`** gains catalog-mapping conformance cases.
- **ADR 0037's matrix.** SDK-02, SDK-05, SDK-06, AI-06 and AI-07 cite this ADR. Areas 3, 11 and 12 list it.
- **Areas.**
  - Area 3 (Engine runtime) builds the canonical names and kinds in `goodfetch-protocol`, the naming crate, the six forms of `hello` and their renderers, and the Run trace's forms.
  - Area 11 (Client surfaces) builds the generators, the manifests and their checks, `call`, the markers, `Connection`, and the CLI's and MCP's names.
  - Area 12 (Console) shows each call in its Client's form.
- **Other tickets.**
  - [Collie's page API](https://github.com/goodfetch/automan/issues/472) spells `newCDPSession` and `CDPSession` in TypeScript and Python, `r#continue` in Rust, `continue_` and the `ok` property in Python.
  - [API sketch: type-checked stubs and golden scenarios](https://github.com/goodfetch/automan/issues/483) applies the remaining renames: initialisms (`JSONValue`, `URLMatcher`, `PageAPI`, `AIWorkResult` and the error classes), TypeScript reads on handles as methods (`run.id()`, `run.startedAs()`), and `call` on every handle.
  - [The Run in code](https://github.com/goodfetch/automan/issues/478) spells its TypeScript reads as methods (`run.id()`, `run.startedAs()`).
  - [Sessions, logins, Human pause and secrets in code](https://github.com/goodfetch/automan/issues/476) gets `identities` as a sub-object in every language, and its cookie operations kept off agent surfaces.
  - [Agent-writability check](https://github.com/goodfetch/automan/issues/484) runs pyright strict, mypy with `--enable-error-code deprecated` and oxlint's `no-deprecated`.
  - [Docs site and positioning of two Products](https://github.com/goodfetch/automan/issues/499) shows each reference page's canonical name and per-language spellings.
- **The `prototype/sdk-api` sketch** holds this ADR's signatures in its `catalog` files for each language ([`b9eaed1`](https://github.com/goodfetch/automan/commit/b9eaed1), [`251bda0`](https://github.com/goodfetch/automan/commit/251bda0)). The pain coverage matrix closes P07, P33, P43 and K06 as API.
