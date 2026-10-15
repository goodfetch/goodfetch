# Hand-written members

ADR 0099 generates every member a catalog kind expresses, and lists every public hand-written member in its SDK's manifest: the catalog entry it serves, or "no entry" with a reason and the canonical words each language spells by its own rule, with the form it takes (a function, a decorator, a type, a method). The Quick suite fails a member missing from another SDK that has its receiver unless its line says why. This is the sketch's manifest, one table for the three SDKs; the SDKs keep one file each.

| Words | TypeScript | Python | Rust | Serves | Form and notes |
|---|---|---|---|---|---|
| `collie` (connection) | `Collie` in `@goodfetch/collie` | `Collie`, `AsyncCollie` in `goodfetch_collie` | `goodfetch_collie::Collie`, `blocking::Collie` | no entry: each Product's connection type (ADRs 0090, 0095) | type with `connect` and, in Rust, `scope` |
| `foxhound` (connection) | `Foxhound` in `@goodfetch/foxhound` | `Foxhound`, `AsyncFoxhound` | `goodfetch_foxhound::Foxhound`, `blocking::Foxhound` | no entry: as above | type with `connect` and, in Rust, `scope` |
| `connection` | `interface Connection` | `Connection`, `AsyncConnection` (base classes) | `trait Connection`, `blocking::Connection` | no entry: what both Products' connections share (ADR 0099) | type |
| `connect` | `Collie.connect(options)` | `Collie.connect(...)`, `AsyncCollie.connect(...)` | `Collie::connect(options)` | `hello` | static method; Foxhound's takes Foxhound's `ConnectOptions` |
| `scope` | none: `await using` | none: `with` / `async with` | `Collie::scope(options, body)` | no entry: Rust has no scoped disposal | associated function, Rust only |
| `connect_options` | `ConnectOptions` | keyword arguments of `connect` | `ConnectOptions` (each Product's) | `hello` and the SDK-handler registrations | options type; Foxhound's adds `fetchBackends` / `fetch_backend` and `sinks` / `sink` |
| `call` | `call(operation, params?, options?)` on the connection and every handle | `call(operation, params=None, *, timeout=...)` | `call(operation)`, `.params(…)` | the raw catalog call (G8) | method |
| `subscription` | `Subscription` | `Subscription` | none: events are an `EventStream` | no entry: what `on(...)` returns (ADR 0081) | type |
| `event_iterator` | `EventIterator<T>` (`AsyncIterable`) | `EventIterator`, `AsyncEventIterator` | `EventStream<T>`, `blocking::EventIter<T>` | no entry: the sequence kind's shape (ADR 0081) | type |
| `waiter` | none: `waitForX` returns a promise armed on call | `expect_x` blocks | `Waiter<T>` with `during(action)` | no entry: the waiter kind's shape (ADR 0080) | type |
| `call` (builder) | none: a promise | none: a coroutine | `Call<T>`, `Query<T>`, `blocking::Call<T>` | no entry: the round-trip kinds' shape | type |
| `label` (handle) | `Label<I, O>` | `Label[P]` | `Label<P>` | no entry: a crawl's typed Label handle (ADR 0082) | type, made by `crawl.label` |
| `dataset` (handle) | `Dataset<I, O>` | `Dataset[T]` | `Dataset<T>` | no entry: a crawl's typed Dataset handle (ADR 0103) | type, made by `crawl.dataset` |
| `schema` | `Schema` (Standard Schema or a JSON Schema) | Pydantic models, `JSONSchema` | `Schema<T>` (`Schema::of`, `Schema::json`) | no entry: the schema adapters (ADR 0082) | type |
| `secret` | `secret(name)` | `secret(name)` | `secret(name)` | no entry: builds a Secret placeholder (ADR 0092) | function |
| `seed`, `sitemap` | `seed(url, label, payload?)`, `sitemap(url, rules)` | `seed(url, label, payload)`, `sitemap(url, rules)` | `Seed::new`, `Seed::sitemap` | `crawl.start`'s seeds (ADR 0093) | functions in TypeScript and Python, constructors in Rust |
| `expect` | `expect(target)` | `expect(target)` | `expect(target)` (`Expectable`) | the catalog's `expect` matchers (ADR 0091) | function |
| `agent_tool` | `agentTool(...)` | `@agent_tool` | `AgentTool::new(...)` | no entry: an Agent tool's constructor (ADR 0100) | function, decorator, type with `new` |
| `goodfetch_error` and its groups | `GoodfetchError`, `ProgramError`, … | the same | `Error`, `ErrorGroup`, `ErrorDetails` | no entry: the error base classes (ADR 0079) | types |
| `error_code` | a string literal type | a `Literal` type | `ErrorCode`, an open newtype with Collie's codes as constants, Foxhound's in `goodfetch_foxhound::error_code` | the catalog's Error codes (ADR 0099) | type |
| `lend_to_playwright` | `lendToPlaywright(page, chromium)` | `lend_to_playwright(page, chromium)`, sync or async by overloads | none: Rust has no supported CDP library (ADR 0089) | `page.lend` (ADR 0089) | function of the client package |
| `lend_to_puppeteer` | `lendToPuppeteer(page, puppeteer)` | none: Puppeteer is TypeScript's | none | `page.lend` | function, TypeScript only |
| `lend_to_browser_use` | none: Browser Use is Python's | `lend_to_browser_use(page, BrowserSession)` | none | `page.lend` | function, Python only, async |
| `test_engine` | `TestEngine` in each Product's `/testing` | `TestEngine`, `AsyncTestEngine` | `TestEngine` behind `test-util` | no entry: the testing kits (ADRs 0069, 0105) | type; Foxhound's adds `runHandler` / `run_handler` and `runHook` / `run_hook`, the testing kit's `reconcile` |
| `scripted_model` | `scriptedModel(replies)` | `ScriptedModel(replies)` | `ScriptedModel::new(replies)` | no entry: ADR 0069's Scripted model | function, type |
| `fake_error` | `fakeError(class, init)` | `fake_error(class, ...)` | `Error::fake(code, message)` | no entry: an error for a program's own tests | function |
| `for_each` | none: `page.route(url, handler)` keeps its handler | none: as TypeScript | `ctx.for_each(stream, handler)` on a crawl's page-stage and handler contexts, `blocking` too | no entry: hands a page stream to `crawl.serve` for the rest of the attempt (ADR 0111) | method, Rust only |
| `keep` (registration) | none: an init script stays until disposed | none: as TypeScript | `Registration::keep()` | no entry: keeps an init script until its page or Session closes (ADR 0111) | method, Rust only |
