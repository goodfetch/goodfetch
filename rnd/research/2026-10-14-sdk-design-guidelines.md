# Multi-language SDK design guidelines from the leaders: what they prescribe and what it means for Goodfetch

State observed on **2026-10-14**. This note answers [Research: multi-language SDK design guidelines from the leaders](https://github.com/goodfetch/automan/issues/464), a ticket of the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461). It sets out what the leaders of multi-language SDK design prescribe, and how that applies to one Rust engine with thin TypeScript, Python and Rust SDKs speaking JSON-RPC to it, with SDK types generated from Rust types (ADRs [0011](../../docs/adr/0011-one-operation-catalog-for-every-agent-surface.md), [0024](../../docs/adr/0024-engine-binary-with-thin-json-rpc-clients.md), [0038](../../docs/adr/0038-contracts-from-rust-types-one-error-cancellation-and-concurrency-model.md)).

It complements the [consumer API research](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md), which covers the browser drivers, AI browser tools and crawl frameworks that Goodfetch's users compare it with. That note asks *what users of our category like and suffer*. This one asks *what general SDK practice says*, from five sources:

1. the Azure SDK design guidelines (general, TypeScript, Python, Rust);
2. the Stainless-generated SDKs of OpenAI and Anthropic;
3. the Rust API Guidelines and current async Rust SDK practice (aws-sdk-rust, azure_core and others);
4. Python and TypeScript idioms in well-regarded libraries, where they bear on an SDK's API (httpx, Pydantic v2, asyncio, Zod 4, Standard Schema, `await using`, `AbortSignal`);
5. published guidance and evaluations on what makes code that coding agents write correct the first time.

Sections 1 to 5 report findings, each with its source. Section 6 compares the rules for consistency across languages. Section 7 merges user pain. **Section 8, "Lessons for Goodfetch's SDK API", is analysis**, mapped to the map's tickets. Section 9 lists what could not be verified.

## Method and limits

- **Primary sources only:**
  - guideline source files read through the GitHub API, with the last-commit date of each page;
  - SDK source at pinned release tags;
  - official docs, PEPs, TC39 notes and repositories, release notes;
  - papers, opened at their arXiv pages.

  Vendor blog posts are used only for what the vendor itself says or measured, and are labelled as vendor evaluations.
- **User pain** comes from each repository's issue tracker, ranked by reactions with the GitHub search API on 2026-10-14. Counts are a snapshot. A handful of Hacker News threads are cited as anecdotal evidence and labelled that way.
- **Pinned versions:**

  | Source | Version or commit | Date |
  |---|---|---|
  | Azure SDK guidelines (`Azure/azure-sdk` `docs/`) | per page, see §1 | 2019-08-05 to 2026-05-19 |
  | `azure_core` (Rust) | 1.1.0, commit 9a8a5f7 | 2026-07-10 |
  | `azure-core` (Python) | 1.41.0 | 2026-05-07 |
  | `@azure/core-lro`, `core-paging`, `core-rest-pipeline`, `logger`, `abort-controller` | 3.4.1, 1.7.0, 1.25.0, 1.4.0, 2.2.0 | n/a |
  | openai-python | v3.31.0 | 2026-10-14 |
  | openai-node | v7.33.0 | 2026-10-14 |
  | anthropic-sdk-python | v1.13.0 | 2026-10-09 |
  | anthropic-sdk-typescript | sdk-v0.133.0 | 2026-10-09 |
  | Rust API Guidelines | commit 97a0969cb0 | 2025-07-08 |
  | aws-sdk-rust, smithy-rs | `release-2026-10-13` (aws-sdk-s3 1.152.0), `release-2026-09-30.2` | as tagged |
  | Tokio, tokio-util | 1.53.3, 0.7.20 | n/a |
  | httpx | 0.28.1 (1.0 still a pre-release) | 2024-12-06 |
  | Pydantic | 2.14.1 | 2026-10-11 |
  | Zod | 4.6.7 | 2026-10-12 |
  | `@standard-schema/spec` | 1.1.0 | 2025-12-15 |
  | Playwright | 1.64.1 | 2026-10-14 |

- **Limits:**
  - No code sample was executed.
  - Reaction counts on the Azure and Anthropic trackers are low, so their pain rankings are weak.
  - The evidence on coding agents is strong on stale knowledge, how often a model has seen an API, and type-checker feedback. It is **absent** on overloads, builders and fluent chaining: no primary study was found.
  - Each subsection's caveats are collected in §9.

## 1. Azure SDK design guidelines

Azure publishes one set of general guidelines plus one per language. The source lives in [`Azure/azure-sdk` `docs/`](https://github.com/Azure/azure-sdk/tree/main/docs), and each rule has a stable ID that is also an anchor on the rendered page (`https://azure.github.io/azure-sdk/<page>.html#<id>`). Rules come in five strengths: ✅ DO, ⛔️ DO NOT, ☑️ YOU SHOULD, ⚠️ YOU SHOULD NOT and ✔️ YOU MAY. Below they are written MUST, MUST NOT, SHOULD and MAY.

Last commit of each page:

| Page | Last commit |
|---|---|
| [general introduction](https://azure.github.io/azure-sdk/general_introduction.html) | 2019-08-05 |
| [general design](https://azure.github.io/azure-sdk/general_design.html) | 2025-04-03 |
| [general implementation](https://azure.github.io/azure-sdk/general_implementation.html) | 2025-07-30 |
| [compatibility](https://azure.github.io/azure-sdk/general_compatibility.html) | 2026-03-27 |
| [TypeScript design](https://azure.github.io/azure-sdk/typescript_design.html) | 2025-08-04 |
| [Python design](https://azure.github.io/azure-sdk/python_design.html) | 2026-04-28 |
| [Rust introduction](https://azure.github.io/azure-sdk/rust_introduction.html) | 2026-05-19 |
| [Rust implementation](https://azure.github.io/azure-sdk/rust_implementation.html) | 2026-04-15 |

### 1.1 Principles

The same five principles appear word for word in the general, TypeScript, Python and Rust introductions ([general_introduction.html#design-principles](https://azure.github.io/azure-sdk/general_introduction.html#design-principles)). Productivity comes first; completeness, extensibility and performance are "important but secondary".

- **Idiomatic.** The SDK should feel natural to a developer in the target language, and embrace its ecosystem with its strengths and its flaws.
- **Consistent.** Consistent within the language, with the service, and across languages. When these conflict, **"consistency within the language is the highest priority"** and consistency across languages is the lowest. Service-agnostic concepts (logging, HTTP, errors) stay the same everywhere. Feature parity across languages matters more than parity with the service. The whole should feel like one product from one team.
- **Approachable.** Predictable defaults that implement best practices. "Think about progressive concept disclosure". Core use cases must be discoverable.
- **Diagnosable.** It is discoverable when a network call happens and what the defaults are. Error messages are concise, correlated with the service and actionable. Debuggers are easy to integrate.
- **Dependable.** Breaking changes harm users more than most features help them. Avoid dependencies that could force a compatibility break.

### 1.2 Client construction and options

**Clients**

- **General rules:**
  - Client types end in `Client` (`general-client-naming`).
  - The main clients live in the root namespace.
  - A client can be built from the minimum connection and auth information (`general-client-minimal-constructor`).
  - Keep the number of clients small. Several are justified for separate use cases (authoring and runtime), for a "hero" subset, or for different permissions.
- **Configuration** ([general_implementation.html](https://azure.github.io/azure-sdk/general_implementation.html)):
  - Two clients of the same type can have different configurations (`general-config-for-different-clients`).
  - Users can opt out of all global configuration (`general-config-optout`).
  - Behaviour never changes after the client is built (`general-config-behaviour-changes`).
  - Child clients inherit the parent's configuration.
- **Python:**
  - Positional binding parameters and credential, then keyword-only settings (`python-client-constructor-form`).
  - A `from_connection_string` classmethod.
  - Clients are immutable (`python-client-immutable`).
  - One client type per level of the resource hierarchy. Each level can be built directly or through its parent, and `get_<child>_client` makes no network call (`python-client-hier-vend`).
- **TypeScript:**
  - Overloaded constructors. A static `from…` factory only when an overload would be ambiguous (`ts-use-constructor-overloads`).
  - Sub-clients come from `get…Client()`.
- **Rust:**
  - `new(endpoint, credential, options: Option<XClientOptions>) -> Result<Self>`. Other credential types get a `with_{credential_type}` constructor.
  - All methods are thread-safe, usually because the client is immutable and stateless.
  - Clients are exported from a `clients` module and models from a `models` module. There is no prelude.
  - Sub-clients have no public constructors and come from a parent through a method ending in `_client`. That method is not async, and the sub-client clones the parent's pipeline (`rust-subclients-*`).

**Options**

- **General:**
  - Optional parameters go into an `<Operation>Options` bag.
  - **Do not hard-code the service's defaults**: leave unset optional parameters out of the request (`general-params-server-defaults`).
  - Validate client parameters, but **do not validate service parameters**; let the service do it (`general-params-server-validation`).
- **TypeScript:**
  - Options types are named `<ClassName>Options` and `<methodName>Options`.
  - The cancellation option is named `abortSignal`.
  - Durations carry a unit suffix, such as `timeoutInMs` (`ts-options-suffix-durations`).
- **Python has no options objects** (`python-client-options-naming`):
  - Optional arguments are keyword-only.
  - Per-call arguments override the client's defaults and reuse the constructor's names.
  - Every method accepts `timeout`, `headers`, `client_request_id` and `response_hook`.
  - No `**kwargs` for arguments the method itself consumes.
- **Rust:**
  - `XClientOptions` has public `Option<T>` fields, a `client_options: ClientOptions` field, and derives `Clone` and `Default`.
  - It reads **no environment variables or config files** (`rust-client-configuration-env`), because WASM and IoT targets may lack them.
  - Method options are named client + method + `Options` (for example `SecretClientSetSecretOptions`). They are passed as `Option<…>` in the last parameter **even when empty**, so the signature never has to change.
  - Callers fill options with struct literals ending in `..Default::default()`.
  - Builders are only for special cases (`rust-builders-support`); `new()` is preferred.

### 1.3 Errors

- **General:**
  - Prefer exceptions over returned error values (`general-errors-prefer-exceptions`).
  - Any status the service does not define as success becomes an error.
  - The error carries the request and the response (`general-errors-include-request-response`), and rich service details in their own fields.
  - **No new error type unless the user can take a different remedial action** (`general-errors-no-new-types`).
  - **When a configuration setting would fix the error, the message names it** (`azurecore-http-logging-configuration`).
- **Python:**
  - Raise when the task fails, including when no response arrived.
  - But **do not raise for "normal responses"**: an `exists` call must tell a 404 apart from a failure to make the request (`python-errors-normal-responses`).
  - Chain exceptions.
  - The azure-core 1.41.0 tree is `AzureError` → `ServiceRequestError` / `ServiceResponseError` (each with a timeout subclass) → `HttpResponseError` (`status_code`, `reason`, `response`, a structured `error`). Below that sit `ResourceNotFoundError`, `ResourceExistsError`, `ClientAuthenticationError` and others.
- **TypeScript:**
  - Validation failures use built-in `TypeError` and `RangeError`.
  - Catch blocks should **check `name` rather than `instanceof`** (`ts-error-use-name`), because a bundled duplicate of a class breaks `instanceof`.
  - The concrete type is `RestError` with `code`, `statusCode`, `request`, `response` and `details`.
- **Rust:**
  - One `azure_core::Error` with a deliberately coarse `ErrorKind`: `HttpResponse { status, error_code, raw_response }`, `Connection`, `Io`, `DataConversion`, `Credential`, `Other`.
  - Methods include `kind()`, `http_status()`, `with_context` and `downcast_ref`, and the error captures a backtrace.
  - A crate may add its own error, preferably through `TryFrom<azure_core::Error>`.
  - In 1.1.0, `ErrorKind` is **not** `#[non_exhaustive]`.

### 1.4 Long-running operations, pagination, cancellation

**Long-running operations**

- **The poller** ([general_design.html#long-running-operations](https://azure.github.io/azure-sdk/general_design.html#long-running-operations)):
  - Methods start with `begin` (or `start`).
  - They return a poller exposing state, completion, cancel (where the service supports it), stop polling, manual polling and progress.
  - A poller can be rebuilt from another poller's serialized state (`general-lro-continuation`).
  - **A cancellation token cancels only the polling, never the service operation** (`general-lro-cancellation`).
- **TypeScript:** `PollerLike` in `@azure/core-lro` 3.4.1 extends `Promise<TResult>`, so `await poller` works. It also has `pollUntilDone({abortSignal})`, `onProgress`, `serialize()` and `restoreFrom`.
- **Python:** `LROPoller` provides `result(timeout)`, `wait`, `done`, `add_done_callback`, `continuation_token()` and `from_continuation_token()`.
- **Rust:** `Poller<M, F>` in azure_core 1.1.0 implements both `futures::Stream` (progress) and `IntoFuture` (the final response). No public resume API was found (§9).

**Pagination**

- **General:**
  - Use the language's standard iterators over the items themselves (`general-pagination-use-async-iterators`).
  - A list that does not page looks the same as one that does.
  - No "collect into an array" API.
  - A continuation token can resume the listing on another machine.
- **Per language:**
  - TypeScript: `PagedAsyncIterableIterator` with `.byPage({continuationToken})`.
  - Python: `ItemPaged` with `by_page(continuation_token=…)`, and a `list_` prefix.
  - Rust: `Pager<T>` implements `futures::Stream`, with `into_pages()`, and resumes through `PagerOptions.continuation`.

**Cancellation and timeouts**

- **General:**
  - Every async call accepts the platform's native cancellation token (`general-network-accept-cancellation`), and retries honour it.
  - Cancellation forms a tree: **a child can time out sooner than its parent but cannot extend it**.
- **Per language:**
  - TypeScript: `AbortSignalLike`.
  - Python: a `timeout` keyword argument in seconds on sync methods; async methods are cancelled with `Task.cancel`.
  - Rust: **cancel by dropping the `Future`**, with no tokens.

### 1.5 Logging, tracing and telemetry

- **Logging:**
  - Log handlers are pluggable, and the SDK takes no dependency on a concrete logging framework.
  - Warning means the function failed its task. Self-healing retries are not warnings.
  - Headers and query parameters are logged only from an allow-list; everything else is redacted.
  - Production default is WARNING.
  - **Log output is treated as a compatibility contract**: new entries in a minor release, other changes behind an opt-in ([general_compatibility.html](https://azure.github.io/azure-sdk/general_compatibility.html)).
- **Tracing:** OpenTelemetry, with **one span per public method**. Spans from nested SDK calls are suppressed, and W3C `traceparent` is propagated.
- **Per language:**
  - TypeScript: the `debug` module, writing to stderr, with channels `azure:<service>:<level>`.
  - Python: the standard `logging` module, with loggers named after the module.
  - Rust: `tracing` may be used for logs but not for distributed tracing. Default to WARN, honour `RUST_LOG`, never trace personal data, and prefer a redacting `SafeDebug` over `Debug`.

### 1.6 Sync and async

- **General:** support both, except where the language or its default runtime does not (`general-network-support-sync-and-async`).
- **Python:**
  - Separate sync and async classes **with the same names**, the async ones in a `.aio` namespace.
  - Models are not duplicated.
  - aiohttp is the default async stack.
- **TypeScript:** async only.
- **Rust:**
  - Async only (`rust-client-methods-async-nosync`); users who need sync call `futures::executor::block_on`.
  - **No unconditional dependency on a runtime or HTTP stack.** Tokio and reqwest are default features.
  - Async traits produce `Send` futures (`?Send` on wasm32).

### 1.7 Versioning

- **Service versions:**
  - Stable SDKs target only GA service versions, with the latest GA as the default.
  - The version can be chosen at construction and is typed as a `ServiceVersion` enum.
  - In Python, changing the default API version bumps at least the minor version.
- **Compatibility** ([general_compatibility.html](https://azure.github.io/azure-sdk/general_compatibility.html)):
  - SDKs are at least as compatible as the language's standard library.
  - Binary-breaking changes are never permitted in a minor or a patch.
- **TypeScript:** forbids a major version 0 (`ts-versioning-no-version-0`).
- **Support** ([policies_support.html](https://azure.github.io/azure-sdk/policies_support.html)):
  - Each major stays active at least 12 months.
  - A deprecated major gets critical and security fixes for at least 12 more months.

### 1.8 Rust model types

- **Results:**
  - Methods return `Result<Response<T, F>>`.
  - The raw status, headers and body are always reachable (`into_body()` gives bytes, `into_model()` deserializes).
- **Models:**
  - Models derive `Clone` and `Default`, use serde, and have `pub` fields that are all `Option<T>`, skipped when `None`.
  - `#[non_exhaustive]` goes on **response-only** structs and **never on request structs**, because it blocks `..Default::default()` construction.
- **Enums:**
  - Enums are **never** `#[non_exhaustive]`: an extensible enum gets an `UnknownValue` variant instead (rationale in Azure/typespec-rust#809, not read).

### 1.9 Where Azure's guidelines and code disagree

- **Rust `begin_` and `list_` methods:** the guidelines call them async, but the shipped code makes the call itself sync and awaits the returned poller (`begin_create_certificate(...)?.await?`, `list_secret_properties(None)?`).
- **`byPage` settings in TypeScript:** the guideline allows no `byPage` setting except the continuation token, yet `@azure/core-paging` 1.7.0 ships `maxPageSize`.
- **Undocumented helpers:** `PollerLike` and `@azure/logger` are not named in the guidelines. The guidelines tell SDKs to use `debug` directly.

## 2. Stainless-generated SDKs (OpenAI and Anthropic)

Four repositories were read at their latest tags (see Method).

Paths below are relative to these tag roots:

| Prefix | Root |
|---|---|
| OP | `https://github.com/openai/openai-python/blob/v3.31.0/` |
| ON | `https://github.com/openai/openai-node/blob/v7.33.0/` |
| AP | `https://github.com/anthropics/anthropic-sdk-python/blob/v1.13.0/` |
| AT | `https://github.com/anthropics/anthropic-sdk-typescript/blob/sdk-v0.133.0/` |

The Anthropic READMEs now point to platform.claude.com, so Anthropic facts come from source, `helpers.md`, `tools.md` and `MIGRATION.md`.

### 2.1 No Stainless Rust target

- **Targets:** the Stainless docs index lists TypeScript, Python, Go, Java, Kotlin, Ruby, PHP, C# and SQL ([stainless.com/docs](https://www.stainless.com/docs)). Its Fern comparison page marks Rust and Swift as "Planned" (read through a summarizer, §9).
- **No generated Rust SDK found:** a GitHub search found no Stainless-generated Rust SDK, so there is no Stainless Rust shape to copy.
- **Python decisions:** Stainless's Python page describes opt-in breaking changes called "Editions". It generates enums as `Literal` types rather than `Enum`, so that new values stay forward compatible ([stainless.com/docs/sdks/python](https://www.stainless.com/docs/sdks/python/)).

### 2.2 Shape

**Sync and async**

- Python has two classes with identical method names: `OpenAI` / `AsyncOpenAI` (OP `README.md`, "otherwise identical") and `Anthropic` / `AsyncAnthropic`.
- Both clients are context managers (`with`, `async with`, or an explicit `close()`).
- aiohttp is an optional backend.
- In 2026 both Python SDKs moved from httpx to httpx2 (OP `httpx2.md`; AP `MIGRATION.md`).
- TypeScript has one async client.

**Errors**

- **Python tree:** `APIError(message, request, body, code, param, type)`, then:
  - `APIStatusError`, with `response`, `status_code` and `request_id`, and one subclass per status from `BadRequestError` to `InternalServerError`;
  - `APIConnectionError`, with `APITimeoutError` under it.

  Source: OP `src/openai/_exceptions.py#L46-L162`.
- **Anthropic additions:**
  - `type`, `request_id` and `workspace_id` on status errors.
  - `OverloadedError` (529), `RequestTooLargeError` (413) and `DeadlineExceededError`.
  - A `RetryableError` that middleware raises to opt a request into retries (AP `src/anthropic/_exceptions.py`).
- **Stream failures:** transport failures while consuming a stream are wrapped in SDK types, with the original exception as the cause.
- **TypeScript:** one generic `APIError<TStatus, THeaders, TError>` (`status`, `headers`, `error`, `code`, `requestID`). User aborts get their own class, `APIUserAbortError`, and timeouts get `APIConnectionTimeoutError` (ON `src/core/error.ts`).

**Retries**

- **Defaults:**
  - 2 retries by default, on connection errors, 408, 409, 429 and 5xx.
  - Backoff starts at 0.5 s, doubles, caps at 8 s, minus up to 25% jitter.
  - `retry-after-ms`, then `retry-after`, is honoured.
- **Server overrides:**
  - An `x-should-retry` header overrides the status rules.
  - OpenAI Python honours `retry-after` only up to 120 s and does not retry beyond that. Anthropic Python honours any positive value.
- **What is never retried:**
  - Requests are retried only when the body can safely be resent.
  - **Stream consumption is never retried**, so output is not duplicated (OP `src/openai/_base_client.py#L812-L930`; OP `README.md`).
- **Idempotency keys:** the base client can send an idempotency key reused across retries, but neither vendor turns it on.

**Timeouts**

- **Python:** 600 s by default with a 5 s connect. It accepts a float or a granular `Timeout`, per client or per call.
- **TypeScript:** 600 000 ms by default.
- **Anthropic's long-request guard:** both Anthropic SDKs refuse a non-streaming request whose expected duration exceeds 10 minutes, estimated from `max_tokens`. The error message says streaming is required (AP `src/anthropic/_base_client.py#L788-L801`; AT `src/client.ts#L1683-L1693`). The guard applies only when the caller left the timeout at its default.

**Streaming and cancellation**

- **Python:** `stream=True` returns an iterable `Stream` / `AsyncStream` that is a context manager with `close()`.
- **TypeScript:** `for await` over a `Stream` that owns an `AbortController`. Breaking out of the loop aborts the request, and iterating a consumed stream throws (use `.tee()`).
- **Higher-level helpers:** event accumulation with final values (`get_final_message()`, `.finalMessage()`), a `text_stream` lens, and TypeScript event emitters (`.on('text' | 'message' | 'error' | 'abort' | 'end')`).
- **Cancellation:**
  - TypeScript takes a per-request `signal: AbortSignal` that also covers reading the body (ON `README.md`; ON `src/internal/request-options.ts#L65`).
  - **Python has no per-request cancellation token**; you close the stream or cancel the task. Open issue [openai-python#2643](https://github.com/openai/openai-python/issues/2643) asks for it.

**Raw access and request IDs**

- **Python:** `.with_raw_response.<method>()` returns headers plus `.parse()`, and `.with_streaming_response` reads the body lazily inside a context manager.
- **TypeScript:** `.asResponse()` and `.withResponse()` return `{data, response, request_id}`.
- **Request IDs:** every response model carries `_request_id`, and every status error carries `request_id`.
- **Undocumented endpoints:** `client.get/post(...)` reach them, still with retries.

**Per-call options and sentinels**

- **Cloning:** `client.with_options(...)` (Python) and `client.withOptions(...)` (TypeScript) return a modified clone; the original is untouched.
- **Per-call arguments:**
  - Python: `extra_headers`, `extra_query`, `extra_body` and `timeout`.
  - TypeScript: a trailing `RequestOptions` with `headers`, `query`, `body`, `maxRetries`, `timeout`, `signal` and `idempotencyKey`.
- **Three states in Python:**
  - A `NotGiven` sentinel separates "absent" from an explicit `None` (so `timeout=None` means no timeout).
  - An `Omit` sentinel drops a key.
  - Generated body parameters default to `omit`, so `None` serializes as JSON `null` (OP `src/openai/_types.py#L133-L187`).
  - On responses, `model_fields_set` tells null from missing.

**Pagination**

- **Auto-pagination:** `for item in client.x.list()` (and `async for` / `for await`) pages automatically.
- **Manual control:** `has_next_page()`, `get_next_page()`, `iter_pages()`.
- **TypeScript:** a `PagePromise` can be iterated straight from `list()` (ON `src/core/pagination.ts`).

**Types and forward compatibility**

- **Python:**
  - Request parameters are `TypedDict`s and responses are Pydantic models with `extra="allow"`, so unknown fields are kept (`model_extra`).
  - **Responses are not validated by default**: they are built with `construct`, so new enum values pass through (OP `src/openai/_models.py`; `_base_client.py#L747-L752`).
- **TypeScript:** extra response properties are neither validated nor stripped.
- **Structured output:**
  - `parse()` helpers take Pydantic models in Python and Zod schemas in TypeScript (`zodResponseFormat`, `zodOutputFormat`, `betaZodTool`).
  - Zod is a peer dependency accepting `^3.25 || ^4.0`.
  - Both TypeScript SDKs also ship Standard Schema helpers, with types copied from `@standard-schema/spec` 1.1.0 (AT `src/lib/standard-schema.ts`).

**Logging and environment**

- **Python:** standard `logging`, switched on by `OPENAI_LOG` / `ANTHROPIC_LOG`.
- **TypeScript:**
  - The `logLevel` option wins over the environment variable. The default is `warn`.
  - A custom `logger` can be passed.
  - Debug logs redact auth headers.
- **Environment defaults:** API keys and base URLs default from environment variables (`OPENAI_API_KEY`, `ANTHROPIC_BASE_URL`, …).

**Versioning**

- **OpenAI's policy:** SemVer, except that some changes can ship in minors:
  - type-only changes;
  - undocumented internals;
  - changes unlikely to affect most users.

  Raising the minimum Python version also ships as a minor (OP and ON `README.md`, "Versioning").
- **Anthropic:** the Python SDK went to 1.0 with a migration guide. The TypeScript SDK is still 0.x.

**Helpers and laziness**

- **Tool runners:** they loop over model calls and run tools (`client.beta.messages.tool_runner()`; TypeScript `toolRunner` forwards an `AbortSignal` to each tool's `run`).
- **TypeScript `APIPromise` sends eagerly and parses lazily**: the HTTP call starts when the method is called, and the body is parsed only when awaited (ON `src/client.ts#L1411-L1422`, `src/core/api-promise.ts`). A Rust future, by contrast, does nothing until it is polled.

## 3. Rust API Guidelines and async Rust SDK practice

### 3.1 Rust API Guidelines

The [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) were last changed on 2025-07-08 (commit 97a0969cb0, a link fix). They **never mention async, futures, streams, cancellation or `#[non_exhaustive]`**, so async SDK guidance has to come from practice (§3.3 to §3.6). The rules that bear on an SDK:

| ID | Rule |
|---|---|
| [C-CASE](https://rust-lang.github.io/api-guidelines/naming.html#c-case), [C-CONV](https://rust-lang.github.io/api-guidelines/naming.html#c-conv), [C-GETTER](https://rust-lang.github.io/api-guidelines/naming.html#c-getter) | RFC 430 casing; `as_` (free), `to_` (costly), `into_` (consuming); getters have no `get_` prefix |
| [C-COMMON-TRAITS](https://rust-lang.github.io/api-guidelines/interoperability.html#c-common-traits), [C-SERDE](https://rust-lang.github.io/api-guidelines/interoperability.html#c-serde) | Implement common traits eagerly (downstream crates cannot add them); data types implement serde, behind a feature named exactly `serde` if optional |
| [C-SEND-SYNC](https://rust-lang.github.io/api-guidelines/interoperability.html#c-send-sync) | `Send` and `Sync` where possible, with compile-time assertions in tests |
| [C-GOOD-ERR](https://rust-lang.github.io/api-guidelines/interoperability.html#c-good-err) | Errors implement `std::error::Error` and are `Send + Sync + 'static`; Display lowercase, no trailing punctuation |
| [C-EXAMPLE](https://rust-lang.github.io/api-guidelines/documentation.html#c-example), [C-QUESTION-MARK](https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark), [C-FAILURE](https://rust-lang.github.io/api-guidelines/documentation.html#c-failure) | Examples on every public item, using `?` because users copy them verbatim; "Errors" and "Panics" sections |
| [C-CTOR](https://rust-lang.github.io/api-guidelines/predictability.html#c-ctor), [C-BUILDER](https://rust-lang.github.io/api-guidelines/type-safety.html#c-builder) | Constructors are static `new`; builders when construction is complex or has side effects such as spawning a process |
| [C-CUSTOM-TYPE](https://rust-lang.github.io/api-guidelines/type-safety.html#c-custom-type), [C-NEWTYPE](https://rust-lang.github.io/api-guidelines/type-safety.html#c-newtype) | Enums instead of `bool` parameters; newtypes for IDs |
| [C-DTOR-FAIL](https://rust-lang.github.io/api-guidelines/dependability.html#c-dtor-fail), [C-DTOR-BLOCK](https://rust-lang.github.io/api-guidelines/dependability.html#c-dtor-block) | Destructors never fail and never block; provide a `close` returning `Result`, with best-effort teardown in `Drop` |
| [C-STRUCT-PRIVATE](https://rust-lang.github.io/api-guidelines/future-proofing.html#c-struct-private), [C-SEALED](https://rust-lang.github.io/api-guidelines/future-proofing.html#c-sealed), [C-NEWTYPE-HIDE](https://rust-lang.github.io/api-guidelines/future-proofing.html#c-newtype-hide) | Public fields are a strong commitment; sealed traits can gain methods; hide representations behind `impl Trait` or wrappers |
| [C-STABLE](https://rust-lang.github.io/api-guidelines/necessities.html#c-stable) | A 1.0 crate's public dependencies must be stable; a `From<dep::Error>` impl leaks the dependency into the API |

### 3.2 `#[non_exhaustive]`

From the [Reference](https://doc.rust-lang.org/reference/attributes/type_system.html) and [RFC 2008](https://rust-lang.github.io/rfcs/2008-non-exhaustive.html):

- **What it does outside the defining crate:**
  - Callers cannot build the struct with a literal, **including `..Default::default()`**.
  - Matches need a wildcard arm.
  - Inside the defining crate it has no effect.
- **What it costs:** downstream code loses the compile-time signal when a variant is added. The lint that would restore it (`non_exhaustive_omitted_patterns`) is still unstable ([rust-lang/rust#89554](https://github.com/rust-lang/rust/issues/89554)).
- **Practice diverges:**
  - Azure puts it on response models only, never on request models or enums (§1.8).
  - AWS puts it on `SdkError` and on every operation error enum (§3.4).

### 3.3 `IntoFuture`: awaiting a builder without `.send()`

- **Stable since Rust 1.64** ([2022-09-22](https://blog.rust-lang.org/2022/09/22/Rust-1.64.0/)). The [std docs](https://doc.rust-lang.org/std/future/trait.IntoFuture.html) call it most useful for "async builder" types: `Multiply::new(0, 0).number(2).factor(2).await`.
- **Adoption is uneven:**

  | Crate | Pattern |
  |---|---|
  | aws-sdk-s3 1.152.0 | Fluent builder plus `.send().await`, no `IntoFuture`. [awslabs/aws-sdk-rust#1027](https://github.com/awslabs/aws-sdk-rust/issues/1027) asks for it: open, 0 reactions. In #1086 a contributor called compile-time typed builders not realistic for the SDK. |
  | azure_core 1.1.0 | `async fn` methods with `Option<…Options>`; `IntoFuture` only on `Poller`. |
  | twilight-http 0.17.1 | `IntoFuture` on every request builder. |
  | teloxide-core 0.13.0 | `IntoFuture` and `send()` both. |
  | octocrab 0.54.4 | `.send().await`. |
  | kube 4.2.0 | Params structs. |
  | async-openai 0.42.2 | derive_builder request structs passed to `async fn`. |
  | bon 3.7.0 and later | `#[builder(derive(IntoFuture(Box)))]` for builders of async functions. |

- **Rust futures are lazy:** tokio-postgres documents that a request is sent only when its future is first polled ([docs.rs](https://docs.rs/tokio-postgres/latest/tokio_postgres/)).

### 3.4 aws-sdk-rust: a mature generated SDK

**Client and configuration**

- **Client:** `Client { handle: Arc<Handle> }` derives `Clone`; cloning is one atomic reference-count update ([best practices](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/best-practices.html)). `Client::from_conf` panics on an invalid configuration.
- **Behaviour versions:**
  - A [`BehaviorVersion`](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/behavior-versions.html) is required, and the constructor panics without one.
  - Older versions are marked `#[deprecated]`, so changed defaults surface as compile warnings.
  - `BehaviorVersion::latest()` opts into the newest defaults.
- **Requests:**
  - A fluent builder then `.send().await`.
  - A missing required field fails at runtime (`ConstructionFailure`), not at compile time.
  - Per-operation overrides go through `.customize().config_override(...)`; the client's own configuration never changes ([per-operation config](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/peroperation.html)).
- **Timeouts and retries:**
  - The default connect timeout is 3.1 s.
  - **There is no default operation or attempt timeout**, and the docs recommend setting both ([timeouts](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/timeouts.html)).
  - Standard retries: 3 attempts with jittered exponential backoff ([retries](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/retries.html)).

**Errors**

- **`SdkError<E, R>`:** every call returns it. It is `#[non_exhaustive]`, with variants `ConstructionFailure`, `TimeoutError`, `DispatchFailure`, `ResponseError` and `ServiceError`.
- **Terse `Display`:** it prints only "service error", by design. Users must wrap it in `DisplayErrorContext` or walk `source()` ([error handling](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/error-handling.html)).
- **Getting the service error:** `into_service_error()` never fails, because non-service errors map into an `Unhandled` variant.
- **Operation error enums** are `#[non_exhaustive]` and provide:
  - `is_no_such_key()`-style helpers;
  - `code()` and `message()`;
  - retryability.

  Their `Unhandled` variant is deprecated for matching, so callers use a guard on `code()`.

**Paginators, waiters, runtime and testing**

- **Paginators:**
  - `.into_paginator().send()` returns AWS's own `PaginationStream`, which **does not implement `futures::Stream`** ([paginating](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/paginating.html)).
  - That was deliberate: smithy-rs#2978 (2023-09-27) removed third-party types from the public API. This is C-STABLE in practice.
- **Waiters:** `client.wait_until_bucket_exists().bucket(..).wait(Duration).await` ([waiters](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/waiters.html)).
- **Runtime:**
  - A pluggable `AsyncSleep`, with Tokio as the default under a feature.
  - The SDK is intended to be runtime-agnostic but recommends Tokio ([concurrency](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/concurrency.html)).
- **Testing:** three supported routes ([testing](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/testing.html)):
  - `mockall`;
  - a static replay client;
  - `aws-smithy-mocks`, behind a `test-util` feature.

### 3.5 Cancellation, drop and cleanup

- **Cancellation is dropping the future:**
  - Tokio's [select tutorial](https://tokio.rs/tokio/tutorial/select) states it, and cleanup belongs in `Drop`.
  - The [`select!` docs](https://docs.rs/tokio/latest/tokio/macro.select.html) define a cancel-safe future as one that can be dropped and recreated as a no-op. `read_exact`, `write_all` and `Mutex::lock` are not cancel-safe.
  - An SDK method should therefore document whether dropping it can leave a request already sent.
- **Explicit tokens:** [`CancellationToken`](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html) (tokio-util 0.7.20) provides `child_token` (a child never cancels its parent), `drop_guard` and `run_until_cancelled`. The [shutdown guide](https://tokio.rs/tokio/topics/shutdown) pairs it with `TaskTracker`.
- **Spawned tasks and processes outlive their handles by default:**
  - Dropping a `JoinHandle` detaches the task.
  - A dropped `tokio::process::Child` keeps running unless `kill_on_drop(true)` is set, and reaping is best-effort ([docs](https://docs.rs/tokio/latest/tokio/process/struct.Command.html#method.kill_on_drop)). This matters for an SDK that spawns the engine.
- **No async `Drop` on stable Rust:**
  - `AsyncDrop` is nightly-only ([rust-lang/rust#126482](https://github.com/rust-lang/rust/issues/126482)).
  - The 2026 project-goals roadmap ([roadmap-just-add-async.md](https://github.com/rust-lang/rust-project-goals/blob/main/src/2026/roadmap-just-add-async.md)) says destructors are sync-only.
  - Async drop depends on a 2026–2027 goal.
  - So, combining C-DTOR-FAIL and C-DTOR-BLOCK: provide an explicit `close().await -> Result`, plus best-effort sync teardown in `Drop`.
- **Runtime-agnostic pattern:** tokio-postgres's `connect()` returns `(Client, Connection)`, and the caller spawns the connection.

### 3.6 Async traits, streams and errors

- **Async traits:**
  - `async fn` in traits shipped in Rust 1.75 (GitHub release 2023-12-28; [announcement](https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits/)).
  - Users cannot add a `Send` bound later, and such traits are not object-safe. `trait_variant::make` is the recommended workaround.
  - Return-type notation is still unstable ([#109417](https://github.com/rust-lang/rust/issues/109417)).
  - Async closures shipped in 1.85 (2025-02-20).
  - `AsyncIterator` is nightly-only, and async iteration is not a 2026 focus.
  - The latest stable is 1.99.0 (2026-10-01).
- **Streams in practice:**
  - async-openai: `Pin<Box<dyn Stream<Item = Result<T, E>> + Send>>`.
  - kube: `impl Stream`.
  - Azure: `Pager` and `Poller` implement `futures::Stream`.
  - AWS: its own type, for C-STABLE.
- **Errors:**
  - [anyhow](https://github.com/dtolnay/anyhow) (1.0.104) is for applications and thiserror is for libraries.
  - [thiserror](https://github.com/dtolnay/thiserror) (2.0.21) does not appear in the public API, and `#[error(transparent)]` suits an opaque error that can evolve.
  - `#[from]` on a dependency's error leaks the dependency.

### 3.7 Builders and options in Rust

| Approach | Required fields checked | Notes |
|---|---|---|
| [bon](https://bon-rs.com/) 3.10.2 | At compile time (typestate) | Works on functions and async functions; `derive(IntoFuture(Box))` |
| typed-builder 0.23.2 | At compile time | Missing fields surface as a deprecation warning; the builder type is not meant to be passed around |
| derive_builder 0.20.2 | At runtime (`build()` returns `Err`) | Owned, mutable or immutable patterns |
| Options struct with `..Default::default()` (Azure) | Required values are positional | Incompatible with `#[non_exhaustive]` on the options struct |
| Fluent builder with `.send()` (AWS) | At runtime | `.send()` is mandatory |

## 4. Python and TypeScript idioms that bear on an SDK API

### 4.1 Python

**httpx** 0.28.1 ([docs](https://www.python-httpx.org/); 1.0 still in pre-release, 1.0.dev6 on 2026-08-31; [#947](https://github.com/encode/httpx/issues/947) open):

- **Sync and async pair:** `Client` and `AsyncClient` share method names. They differ only in `await` and in `aclose()` instead of `close()`, and the async client runs on asyncio or trio.
- **Lifecycle:**
  - The context manager is the recommended way to use a client, because the client reuses connections.
  - In the source, a state enum moves the client from unopened to opened to closed, and reopening raises `RuntimeError`.
- **Three-state options:**
  - Options are keyword-only.
  - A `USE_CLIENT_DEFAULT` sentinel separates "not passed" from `None`, and `timeout=None` explicitly disables the client's default.
- **Timeouts:**
  - The default is 5 s of network inactivity.
  - There are four kinds (connect, read, write, pool), each with its own exception.
  - A timeout can be set per client or per request.
- **Exceptions:**
  - `HTTPError` splits into `RequestError` (transport, timeouts, protocol) and `HTTPStatusError`.
  - **A 4xx or 5xx is not an exception by default**; `raise_for_status()` opts in.
- **Event hooks:** request and response hooks are lists of callables, and they must be async on `AsyncClient`.

**Pydantic** 2.14.1 ([docs](https://pydantic.dev/docs/validation/latest/get-started/)):

- **Core:** the validation core is written in Rust.
- **JSON Schema:** `model_json_schema()` and `TypeAdapter.json_schema()` emit JSON Schema draft 2020-12, in a validation mode and a serialization mode.
- **TypeAdapter:** it covers `TypedDict`, dataclasses and `list[T]`. Build each adapter once and reuse it.
- **Strict mode:** it stops coercion. It can be set per call, per model, per field or by annotation.
- **Extra fields:** `ConfigDict(extra=…)` takes `'ignore'` (the default), `'forbid'` or `'allow'`. For wire types that may gain fields, this choice decides forward compatibility.

**asyncio and structured concurrency** ([asyncio tasks](https://docs.python.org/3/library/asyncio-task.html), [runners](https://docs.python.org/3/library/asyncio-runner.html)):

- **3.11 additions:** `asyncio.timeout()` turns cancellation into `TimeoutError` at the boundary. `TaskGroup` cancels the remaining tasks when one fails and raises an `ExceptionGroup` ([PEP 654](https://peps.python.org/pep-0654/)).
- **Cancellation:** a caught `CancelledError` should be re-raised after cleanup, and code that truly suppresses it must call `uncancel()`.
- **Sync wrappers break inside a running loop:**
  - `asyncio.run()` cannot run while another loop is running in the same thread.
  - CPython raises "asyncio.run() cannot be called from a running event loop" ([runners.py](https://github.com/python/cpython/blob/v3.14.0/Lib/asyncio/runners.py#L201)).
  - A sync facade built on `asyncio.run` therefore breaks in Jupyter and inside async apps.
- **Ctrl-C:** `Runner` installs its own SIGINT handler.
- **AnyIO:** [AnyIO](https://anyio.readthedocs.io/en/stable/) 4.15.1 gives Trio-like structured concurrency on asyncio or Trio.
- **Async generators:** `contextlib.aclosing()` (3.10) closes them.

**Supported Python versions** ([devguide](https://devguide.python.org/versions/)): 3.10 reached end of life on 2026-10-01. 3.11 is the natural minimum: it brings `TaskGroup`, `asyncio.timeout`, `ExceptionGroup` and `typing.Unpack` / `Self` with no backports.

**Typing:**

- **Typed keyword arguments:** [PEP 692](https://peps.python.org/pep-0692/) `Unpack[TypedDict]` types `**kwargs` (Required and NotRequired keys).
- **Deprecation:** [PEP 702](https://peps.python.org/pep-0702/) `@deprecated` (3.13, or via `typing_extensions`) warns in type checkers and at runtime, and applies to individual overloads.
- **Newer generics and core typing:** [PEP 695](https://peps.python.org/pep-0695/) generics (3.12); `Literal` ([PEP 586](https://peps.python.org/pep-0586/)); keyword-only arguments ([PEP 3102](https://peps.python.org/pep-3102/)).
- **Distribution:** a typed package ships `py.typed` ([typing spec](https://typing.python.org/en/latest/spec/distributing.html)).

### 4.2 TypeScript and JavaScript

**Explicit resource management** (`using`, `await using`, `Symbol.asyncDispose`, `DisposableStack`):

- **Standard status:**
  - Listed as a finished Stage 4 proposal, expected in ES2027 ([finished proposals](https://github.com/tc39/proposals/blob/main/finished-proposals.md)).
  - The May 2026 TC39 notes record all conditions met ([notes](https://github.com/tc39/notes/blob/HEAD/meetings/2026-05/may-19.md)).
  - The proposal README still says Stage 3, which is stale.
- **TypeScript:** supported since [5.2](https://www.typescriptlang.org/docs/handbook/release-notes/typescript-5-2.html), with resources disposed in reverse order.
- **Runtimes:**
  - Node 24.0, Chrome 134, Firefox 141 and Deno.
  - Safari only in preview.
  - MDN lists `await using` as limited availability ([MDN](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Statements/await_using)).
- **Node release lines:** Node 20 reached end of life on 2026-04-30. Node 22 is in maintenance until 2027-04-30, and Node 24 enters maintenance on 2026-10-20.
- **Adoption:**
  - Playwright 1.64.1 declares `[Symbol.asyncDispose]()` on `Browser`, `BrowserContext`, `Page`, `APIRequestContext` and more ([types.d.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/types/types.d.ts)).
  - Since 1.59 (2026-04-01), Playwright APIs return async disposables, as in `await using page = await context.newPage()`.
  - This is the closest precedent for an SDK that owns browsers.

**`AbortSignal`** ([Node globals](https://nodejs.org/api/globals.html)):

- **Combinators:** `AbortSignal.timeout()` (Node 17.3) and `AbortSignal.any()` (Node 20.3).
- **Error names:** a timeout aborts with a `TimeoutError` DOMException and a user abort with `AbortError`; MDN tells them apart by `name`.
- **DOM convention:**
  - Take `{ …, signal }` in a trailing options dictionary.
  - Reject pending promises with the signal's abort reason ([DOM spec](https://dom.spec.whatwg.org/#abortcontroller-api-integration)).

**Zod 4 and Standard Schema:**

- **Zod 4:**
  - Zod 4.0.0 was published on 2025-07-09 and the current version is 4.6.7.
  - It has native `z.toJSONSchema()` ([zod.dev/v4](https://zod.dev/v4)).
  - Its [library-author guide](https://zod.dev/library-authors) says: make `zod` a peer dependency, import only from `zod/v4/core`, and consider Standard Schema first for black-box validation.
- **[Standard Schema](https://standardschema.dev/) v1:**
  - A `~standard` property with `version`, `vendor`, `validate` and optional `types`.
  - The package exports only types, and they may be copied in.
  - 33 libraries implement it (Zod 3.24+, Valibot 1.0+, ArkType 2.0+, Effect Schema, yup, joi, …).
  - 64 libraries accept it (tRPC, TanStack Form and Router, Hono middleware, React Hook Form, FastMCP, `zod-jsonrpc`, …).
- **[Standard JSON Schema](https://standardschema.dev/json-schema):**
  - Adds `~standard.jsonSchema.input()` and `.output()` for draft 2020-12, draft-07 and OpenAPI 3.0.
  - Zod 4.2+, ArkType and Valibot (through an adapter) implement it.
  - That is exactly what an engine contract needs from a user's schema, with no dependency on Zod's major versions.

**Other building blocks:**

- **Language features:** async iteration (`for await`, ES2018) and `Error.cause` (ES2022). MDN suggests `cause` can carry structured data for machines.
- **Loading ESM from CommonJS:** `require(esm)` is unflagged since Node 22.12 and 20.19 ([modules](https://nodejs.org/api/modules.html)). An ESM-only package without top-level `await` can therefore be `require`d on every supported Node line.
- **Absent versus `undefined`:** with [`exactOptionalPropertyTypes`](https://www.typescriptlang.org/tsconfig/#exactOptionalPropertyTypes), an explicit `undefined` differs from an absent property. That gives three states (absent means default, `null` means explicitly off, a value overrides), the TypeScript counterpart of httpx's sentinel.

## 5. Writing APIs for coding agents

### 5.1 Evaluations

**Stale knowledge and API change**

- **Deprecated usage follows context** ([arXiv 2406.09834](https://arxiv.org/abs/2406.09834), ICSE 2025):
  - Setup: 7 models, 8 Python libraries.
  - Deprecated API usage was 25–38% overall.
  - **When the surrounding code already used outdated APIs it rose to 69.7–90.0%, against 9.0–17.8% with up-to-date context.**
- **Docs alone do not teach updates** ([CodeUpdateArena, 2407.06249](https://arxiv.org/abs/2407.06249)): putting an update's documentation in front of open models did not get them to use the change.
- **Imports and version info help** ([VersiCode, 2406.07411](https://arxiv.org/html/2406.07411v2)):
  - GPT-4o reached 65.97 Pass@1 with import statements in context and 44.54 without them.
  - Version information helped less: 52.80 against 49.72.
- **Error feedback helps a little** ([GitChameleon, 2411.05830](https://arxiv.org/abs/2411.05830)): GPT-4o went from 39.9% to 43.7% pass@10 with it.
- **Retrieval helps more than version comments** ([LibEvolutionEval, 2412.04478](https://arxiv.org/html/2412.04478v1), NAACL 2025):
  - A version comment alone added under 1 point.
  - Retrieving version-specific docs added +1.8 to +14.2 F1.
- **Changed behaviour under an unchanged signature is the hardest case** ([RustEvo², 2503.16922](https://arxiv.org/abs/2503.16922)):
  - Success was **38.0%** on changes where behaviour moved without any signature change, against 65.8% on stabilized APIs.
  - It was 32.5% on APIs from after the model's knowledge cutoff, against 56.1% before it.
  - Retrieval added +13.5%.

**Hallucinated and misused APIs**

- **Misuse taxonomy** ([arXiv 2503.22821](https://arxiv.org/html/2503.22821v2)):
  - Hallucination is the most frequent misuse, 23.0–43.3% of failures.
  - Confusion between similar methods is observed but not quantified.
- **Low-frequency APIs** ([CloudAPIBench, 2407.09726](https://arxiv.org/abs/2407.09726)):
  - GPT-4o made **valid calls 93.66% of the time for high-frequency APIs and 38.58% for low-frequency ones**.
  - Documentation lifted low-frequency APIs to 47.94%, but irrelevant retrieved docs hurt high-frequency ones.
- **Hallucinated packages** ([arXiv 2406.10279](https://arxiv.org/abs/2406.10279), USENIX Security 2025): at least 5.2% for commercial models and 21.7% for open models.
- **Prompt wording** ([arXiv 2509.22202](https://arxiv.org/abs/2509.22202)):
  - One-character misspellings in the prompt triggered hallucinated libraries in up to 26% of tasks.
  - Asking for the "latest" approach did so in up to 85%.
- **What static analysis catches** ([arXiv 2604.07755](https://arxiv.org/abs/2604.07755)): it detected 14–85% of library hallucinations.

**Types and compiler feedback**

- **Type-constrained decoding** ([arXiv 2504.09246](https://arxiv.org/abs/2504.09246)):
  - In model-written TypeScript, **94% of compilation errors were type errors and 6% syntax errors**.
  - Constraining generation by types cut compilation errors by 52–75%.
- **Agent interfaces** ([SWE-agent, 2405.15793](https://arxiv.org/abs/2405.15793)):
  - Commands should have few options and concise documentation.
  - Removing the linter guardrail cost 3.0 points.
- **Rust compiler errors** ([RustAssistant, 2308.05177](https://arxiv.org/abs/2308.05177)): a model loop over rustc errors fixed about 74% of real Rust compilation errors.

**Docs and descriptions**

- **Docs versus demonstrations** ([arXiv 2308.00675](https://arxiv.org/abs/2308.00675)): with hundreds of APIs, documentation significantly beat demonstrations.
- **Concise docs** ([EASYTOOL, 2401.06201](https://arxiv.org/abs/2401.06201)): rewriting tool docs into a concise, unified form helped.
- **More text is not automatically better** ([arXiv 2602.14878](https://arxiv.org/abs/2602.14878)): fully augmented MCP tool descriptions raised success by a median 5.85 points, but added 67% more steps and regressed 17% of cases.
- **Parameter schemas matter most for tool selection** ([arXiv 2610.14334](https://arxiv.org/abs/2610.14334), 2026-10-09, very recent): models favour flat top-level parameters over grouped ones.

**Vendor evaluations** (single-vendor, labelled as such)

- **Docs index in AGENTS.md** ([Vercel, 2026-01-27](https://vercel.com/blog/agents-md-outperforms-skills-in-our-agent-evals)): on Next.js 16 APIs missing from training data:
  - no docs scored 53%;
  - a skill scored 53% (79% with explicit instructions);
  - **an 8 KB compressed docs index in `AGENTS.md` scored 100%**.

  The skill was never invoked in 56% of cases.
- **Typed SDK code mode** ([Stainless, 2026-03-02](https://www.stainless.com/blog/sdk-code-mode)): agents writing code against a typed SDK reached 98% completeness on 31 tasks, against 70% for dynamic tools.
- **Code over tool calls** ([Cloudflare, 2025-09-26](https://blog.cloudflare.com/code-mode/); [Anthropic, 2025-11-04](https://www.anthropic.com/engineering/code-execution-with-mcp)): models write code better than they call tools, and presenting tools as TypeScript files cut one example from about 150k to 2k tokens.

**Browser-specific evidence:** no peer-reviewed benchmark of models writing Playwright code was found. Playwright 1.56 ships planner, generator and healer agent definitions and agent-friendly methods (`consoleMessages()`, `pageErrors()`, `requests()`) ([release](https://github.com/microsoft/playwright/releases/tag/v1.56.0)).

### 5.2 Published guidance

- **Anthropic's tool-writing guide** ([Writing effective tools for agents, 2025-09-11](https://www.anthropic.com/engineering/writing-tools-for-agents)):
  - Build few tools, each with a clear, distinct purpose.
  - Namespace by service and resource. Prefix versus suffix had non-trivial effects in their evals.
  - Return meaningful context: readable names next to opaque IDs reduced hallucinations.
  - Be token-efficient: pagination, filtering, truncation.
  - **Errors should state an actionable fix, not an opaque code or traceback.**
  - Use unambiguous parameter names (`user_id`, not `user`) and strict data models.
- **Anthropic's tool docs** ([Define tools](https://platform.claude.com/docs/en/agents-and-tools/tool-use/define-tools)): detailed descriptions are the most important factor. A description says what the tool does, when to use it and when not to, each parameter, and its caveats, and can add input examples.
- **OpenAI's function-calling guide** ([function calling](https://developers.openai.com/api/docs/guides/function-calling)):
  - Use enums and object structure to make invalid states impossible.
  - Combine functions that are always called in sequence.
  - Keep fewer than 20 functions available at the start of a turn.
- **MCP specification** ([2025-11-25 tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)):
  - Tool names are 1–128 characters from `[A-Za-z0-9_.-]`.
  - Execution errors carry actionable feedback so the model can correct itself.
  - `outputSchema` gives typed results.
- **Docs formats for agents:**
  - [llms.txt](https://llmstxt.org/) (2024-09-03): a Markdown index, with a Markdown twin for each docs page.
  - [AGENTS.md](https://agents.md/): the nearest file wins; stewarded by the Linux Foundation's Agentic AI Foundation since 2025-12-09.
  - [Context7](https://context7.com/docs/library-owners): a `context7.json` in the library's repository carries `rules` and `previousVersions`.
- **Practitioner reports (anecdotal):**
  - Models keep writing Tailwind v3 setup ([HN](https://news.ycombinator.com/item?id=47688724)) and Svelte 4 code after those APIs changed ([HN](https://news.ycombinator.com/item?id=45520566)).
  - Rust's compiler is cited as making refactoring with agents easier ([HN](https://news.ycombinator.com/item?id=44591350)).

## 6. Consistency across languages: what is the same, what follows the language

The leaders agree on one rule. **Identical everywhere:** the concepts, verbs, defaults and behaviours. **Per-language idiom:** the mechanism by which each is expressed. Azure states the order explicitly (consistency within the language first, across languages last, §1.1). Stainless and AWS practise it, and Playwright's ports do too (consumer API research, browser drivers §4.3).

| Concern | Same in every language | Expressed per language (TypeScript / Python / Rust) |
|---|---|---|
| Entry point | One small set of clients built from minimal information; configuration fixed after construction; sub-objects created without I/O | Constructor overloads or `from…` factory / constructor with keyword-only settings and `from_…` classmethods / `new(…, Option<Options>)` or a builder |
| Operation names | Same verb for the same operation (`general-client-standardize-verbs`); `begin` for long operations, `list` for paged ones | camelCase / snake_case / snake_case with `into_`/`as_` conversions |
| Optional parameters | Same names and semantics; unset means "let the server decide" | Trailing options object / keyword-only arguments / `Option<…Options>` struct or builder |
| Absent versus null | Three states: absent (inherit), explicit null (disable), value | `exactOptionalPropertyTypes`, `undefined` vs `null` / sentinel (`NotGiven`, `USE_CLIENT_DEFAULT`) / `Option<Option<T>>` or a dedicated enum |
| Lifecycle | Explicit close; scoped disposal recommended | `await using` (`Symbol.asyncDispose`) / `with` and `async with` / `close().await -> Result` plus best-effort `Drop` |
| Sync and async | Async is primary | Async only / two classes with the same method names / async only |
| Cancellation | A tree; a child cannot outlive or extend its parent; retries honour it | `AbortSignal` in options / task cancellation and `asyncio.timeout()` / drop the future, or a `CancellationToken` |
| Durations | One meaning per setting | Milliseconds with a unit suffix (`timeoutInMs`) or a plain `ms` number / float seconds or `timedelta` / `std::time::Duration` |
| Errors | One base, stable machine codes, request context, retryable flag, actionable message naming the setting that fixes it | Error subclass checked by `name` or `code` / exception hierarchy with chaining / an enum or a coarse kind, `std::error::Error + Send + Sync` |
| Collections and streams | Iterate elements, page on demand, resumable continuation | `AsyncIterable` with `byPage()` / iterator with `by_page()` / `Stream`-like with `into_pages()` |
| Long operations | Awaitable handle with state, progress, serialized resume; stopping the wait is distinct from cancelling the work | `PollerLike extends Promise` / `LROPoller.result()` / `Poller: IntoFuture + Stream` |
| Logging | Never stdout; host logging facility; levels mean the same; redaction by allow-list; log output treated as a contract | `debug` or an injected logger / `logging` named loggers / `tracing`, `RUST_LOG` |
| Typed data | Unknown fields and enum values tolerated on results | Plain types, no stripping / Pydantic `extra="allow"`, `Literal` / `#[serde(other)]` or `UnknownValue(String)` |

## 7. User pain with these SDK styles

Reactions as of 2026-10-14. Only issues bearing on API shape are kept.

| # | Pain | Evidence | Style it hits |
|---|---|---|---|
| 1 | **Rust SDK compile time** | [aws-sdk-rust#113](https://github.com/awslabs/aws-sdk-rust/issues/113) (260 reactions, open since 2021; aws-sdk-ec2 took 148.6 s to build) | Large generated Rust crates |
| 2 | **Schema-library majors breaking SDKs** | [openai-node#1576](https://github.com/openai/openai-node/issues/1576) Zod 4 support (90), [#1602](https://github.com/openai/openai-node/issues/1602) (33); Zod's own subpath migration [zod#4371](https://github.com/colinhacks/zod/issues/4371) (98); Pydantic v3 fear [pydantic#10033](https://github.com/pydantic/pydantic/issues/10033) (173) | SDKs depending on a schema library's internals |
| 3 | **No blocking API / async-only friction** | [aws-sdk-rust#505](https://github.com/awslabs/aws-sdk-rust/issues/505) synchronous API (104, open); [openai-python#98](https://github.com/openai/openai-python/issues/98) async requests (46); [httpx#522](https://github.com/encode/httpx/issues/522) keep sync (28) | Async-only or sync-only SDKs |
| 4 | **Mocking and testing the SDK** | [aws-sdk-rust#199](https://github.com/awslabs/aws-sdk-rust/issues/199) (60); [openai-python#398](https://github.com/openai/openai-python/issues/398) (19) | Concrete clients with no test seam |
| 5 | **Everything `Option<T>`** | [aws-sdk-rust#536](https://github.com/awslabs/aws-sdk-rust/issues/536) (52), [#327](https://github.com/awslabs/aws-sdk-rust/issues/327) (31) | Generated Rust models (Azure mandates `Option` on every field) |
| 6 | **Error ergonomics** | [aws-sdk-rust#191](https://github.com/awslabs/aws-sdk-rust/issues/191) error-handling guide (51), [#572](https://github.com/awslabs/aws-sdk-rust/issues/572) three levels of nesting (11); [openai-node#76](https://github.com/openai/openai-node/issues/76) error types (15); [anthropic-sdk-typescript#346](https://github.com/anthropics/anthropic-sdk-typescript/issues/346) "[object Object]" | Generic wrapper errors with terse Display; untyped errors |
| 7 | **Runtime choice** | [aws-sdk-rust#52](https://github.com/awslabs/aws-sdk-rust/issues/52) alternative runtimes (43, open) | Tokio-bound Rust SDKs |
| 8 | **Streaming hangs, idle timeouts and cancellation** | [openai-node#18](https://github.com/openai/openai-node/issues/18) how to stream (54 reactions, 157 comments); [openai-python#2643](https://github.com/openai/openai-python/issues/2643) cancel a stream (11, open); [openai-node#1825](https://github.com/openai/openai-node/issues/1825) timeout covers only time to headers, so body reads can hang; [anthropic-sdk-typescript#867](https://github.com/anthropics/anthropic-sdk-typescript/issues/867), [#998](https://github.com/anthropics/anthropic-sdk-typescript/issues/998) idle watchdog; [openai-python#769](https://github.com/openai/openai-python/issues/769) constant async timeouts (68 comments) | Timeouts that stop at headers; no Python cancel token |
| 9 | **Strict validation turning server drift into exceptions** | [openai-python#1763](https://github.com/openai/openai-python/issues/1763) `parse` raises an unhandled `ValidationError` (13, open); [#2489](https://github.com/openai/openai-python/issues/2489) streaming `ValidationError` after a minor upgrade (11) | Validating results against a schema that lags the server |
| 10 | **Transport-library churn leaking through the SDK** | [openai-python#1902](https://github.com/openai/openai-python/issues/1902) / [#1903](https://github.com/openai/openai-python/issues/1903) httpx 0.28 removed `proxies` (10 and 8); [#3375](https://github.com/openai/openai-python/issues/3375) move to httpx2 (33); [openai-node#816](https://github.com/openai/openai-node/issues/816) ESM/CJS break in a patch; [anthropic-sdk-python#1919](https://github.com/anthropics/anthropic-sdk-python/issues/1919) raw httpx exceptions escaping a stream (open) | Third-party types in the public API (C-STABLE) |
| 11 | **Async teardown** | [azure-sdk-for-python#9060](https://github.com/Azure/azure-sdk-for-python/issues/9060) "Event loop is closed" (11); [httpx#914](https://github.com/encode/httpx/issues/914) (25, open); [azure-sdk-for-python#42152](https://github.com/Azure/azure-sdk-for-python/issues/42152) an `aio` poller blocks the loop (open) | Python async lifecycle |
| 12 | **Exceptions for expected outcomes** | [azure-sdk-for-python#9507](https://github.com/Azure/azure-sdk-for-python/issues/9507) `exists()` (7); [azure-sdk-for-js#28674](https://github.com/Azure/azure-sdk-for-js/issues/28674) 404 thrown and traced as an error; [azure-sdk-for-rust#4707](https://github.com/Azure/azure-sdk-for-rust/issues/4707) an expected 409 logged at ERROR | Throwing and logging normal results as failures |
| 13 | **Resume state wrong** | [azure-sdk-for-python#47972](https://github.com/Azure/azure-sdk-for-python/issues/47972) a resumed poller reports "InProgress" forever (13, open) | Serialized long-operation handles |
| 14 | **Cancellation type identity in bundles** | [azure-sdk-for-js#28059](https://github.com/Azure/azure-sdk-for-js/issues/28059) "Expected signal to be an instanceof AbortSignal" (9) | `instanceof` checks across duplicated packages |
| 15 | **Pre-1.0 churn of the core crate** | [azure-sdk-for-rust#2635](https://github.com/Azure/azure-sdk-for-rust/issues/2635) (4) | Unstable shared core |
| 16 | **Paging settings silently ignored** | [azure-sdk-for-js#20380](https://github.com/Azure/azure-sdk-for-js/issues/20380) (4 reactions, 21 comments) | Options that are accepted but do nothing |

Reaction counts on the Azure and Anthropic trackers are low, so rows 11 to 16 rest on few voices. Rows 1 to 10 are the strongest.

## 8. Lessons for Goodfetch's SDK API (analysis)

**This section is analysis, not decisions.** Each lesson cites the findings above and is mapped to the map's ticket that owns the question. Where a lesson meets an adopted ADR, it says so. ADRs fixed by the map (0011, 0019, 0024, 0028, 0038) are taken as given.

### [#462 SDK API principles](https://github.com/goodfetch/automan/issues/462)

1. **Adopt Azure's precedence order as a stated principle:** idiomatic in each language first, consistent across languages second (§1.1, §6).
   - The catalog (ADR 0011) guarantees the same concepts, verbs and behaviour. Each SDK's hand-written runtime (ADR 0038) expresses them idiomatically.
   - This is already how ADR 0038 splits generated and hand-written code. Naming the rule settles future disputes such as `timeoutInMs` versus `timeout`.
2. **Write Azure's five qualities down as review criteria,** each made checkable. Approachable means progressive disclosure; diagnosable means you can see the call and its defaults. Together with the map's "elegant", they give the reviewer of every ticket a vocabulary.
3. **Treat agent-writability as a design input, not a docs task** (§5).
   - Goodfetch is a new library, so it sits in the low-frequency regime: GPT-4o's valid-call rate fell from 94% to 39% there.
   - It should therefore look like the leaders where the meaning is the same (Playwright, httpx, Stainless).
   - **It must never reuse a familiar name with different behaviour.** Changed behaviour under an unchanged signature is the hardest case for models (38% in RustEvo²).
4. **Keep third-party types out of the public API** (C-STABLE, §3.1; pain 10).
   - No `httpx`, `futures::Stream`, `zod` or `pydantic` type should be the only way to use an operation.
   - Where an ecosystem type is the idiom (Pydantic models in Python), pin a major range and name it in the SDK's compatibility policy (ADR 0030).

### [#465 Entry points and lifecycle](https://github.com/goodfetch/automan/issues/465)

5. **Construction:**
   - The entry object is built from minimal information, with everything else defaulted (Azure `general-client-minimal-constructor`).
   - Its configuration is immutable after construction (Azure, AWS, Stainless).
   - Variants are made by cloning (`with_options` / `withOptions`, AWS `customize()`), never by mutating.
   - Sub-objects (a page from a Session, a run from a crawl) come from their parent without I/O wherever possible, or are visibly async where not.
6. **Scoped disposal in every SDK, plus an explicit close:**
   - TypeScript `await using`: Playwright has returned async disposables since 1.59. It needs Node 24 or a polyfill of `Symbol.asyncDispose`, so `close()` stays first-class.
   - Python `with` and `async with`: httpx calls this the recommended way to use a client, and it raises on reuse after close.
   - Rust `close().await -> Result` with best-effort `Drop` (C-DTOR-FAIL, no async drop before 2026–2027).
7. **The Rust SDK starts an Attached engine, so it must set `kill_on_drop(true)` on the child process** (Tokio's default leaves it running), and it should document what `Drop` does when `close()` was skipped. Python and TypeScript need the same guarantee: an engine that outlives a crashed script is the "Event loop is closed" and zombie-process class of pain (pain 11; consumer API research pain 4).
8. **Evaluate AWS's behaviour version for later.** A required `BehaviorVersion` pins defaults so that new defaults never silently change a running program. Each SDK already ships with one exact engine (ADR 0030) and each run pins what it depends on, such as its filter lists (ADR 0047), so this may be unnecessary in the SDK, but #465 should say so explicitly.

### [#466 Results, outcomes and errors](https://github.com/goodfetch/automan/issues/466)

9. **Exceptions for failures, values for normal outcomes:**
   - Azure prefers exceptions, but forbids raising for normal responses (an `exists` call must not throw on 404) and treats logging an expected 409 at ERROR as a bug (pain 12).
   - For Goodfetch, a page that answered 404 or a Success check that came back false is an outcome, not an exception. Engine, transport or contract failures raise.
   - This also answers the consumer API research's "failures that look like results" pain from the other side.
10. **One error base, one class per Error code (ADR 0038), but check the granularity against Azure's rule:**
    - Add an error type only when the user's remedy differs.
    - Codes can be fine-grained for machines while the class hierarchy groups them by remedy: retryable or not; the user's input, the target site, or the engine. Stainless does exactly this by grouping status codes under connection and status bases.
11. **Fields every error carries:**
    - **Machine data:** the stable code, a retryable flag, the operation, a request or call id (Stainless's `request_id`), and the cause chain (Python chaining, `Error.cause`, Rust `source()`).
    - **A message that names the fix,** and when a setting would fix it, the setting's key. Azure `azurecore-http-logging-configuration`, Anthropic's tool guide and the MCP spec all ask for this. It fits ADR 0028, where every parameter is a setting.
    - **TypeScript:** check errors by `name` or `code`, never only by `instanceof` (Azure `ts-error-use-name`; pain 14).
12. **Rust errors: prefer Azure's single error with a coarse kind plus the code over AWS's generic `SdkError<E>`.**
    - AWS's nesting and terse `Display` are among its top complaints (pain 6).
    - Goodfetch's Error codes are a semver-governed list (ADR 0038, ADR 0030), so the Rust code enum should be `#[non_exhaustive]` or carry an `Unknown(String)` variant. That is how an older Rust SDK survives a newer engine's code.
    - Azure (no `#[non_exhaustive]` on enums, an unknown variant instead) and AWS (`#[non_exhaustive]`) disagree. #466 should pick one.
13. **Raw access and an escape hatch:**
    - Stainless gives every typed call a raw sibling (`with_raw_response`) and a generic `client.get/post`.
    - The Goodfetch counterpart is a typed-but-generic `call(method, params)` on the connection for catalog operations newer than the installed SDK, plus access to the raw JSON-RPC result. It costs little, because the transport is hand-written anyway (ADR 0038).

### [#467 Time, waits, deadlines, cancellation and budgets](https://github.com/goodfetch/automan/issues/467)

14. **Cancellation is a tree in which a child can shorten but never extend its parent's deadline.** Azure states it, and Tokio's `child_token` and ADR 0038's token tree implement it. Map it per language:
    - TypeScript `signal: AbortSignal` in the trailing options (with `AbortSignal.timeout()` and `.any()`);
    - Python task cancellation and `asyncio.timeout()`, plus an explicit timeout argument on the sync client;
    - Rust drop of the future, which **must** send `$/cancel`, documented as cancel-safe or not per method (Tokio `select!` docs).
15. **Distinguish "stop waiting" from "stop the work".** Azure's poller cancellation stops only the polling, never the service operation (`general-lro-cancellation`). ADR 0038 sends `$/cancel` and keeps run-level stop operations separate. The SDK should make both visible: aborting an `await run.done()` must not, by surprise, stop or leave running a crawl.
16. **Unit discipline for durations:**
    - Rust `Duration`.
    - Python float seconds, the convention of httpx, asyncio and Stainless, or `timedelta`.
    - TypeScript milliseconds. Azure asks for a `…InMs` suffix; Stainless and Playwright use a plain `timeout` in milliseconds.

    Pick one per language and never mix (consumer API research: Skyvern's `timeout` in three units).
17. **Deadlines must cover the whole call, not just the time to first byte.** Openai-node's timeout stopping at headers let body reads hang (pain 8). Streams need an idle timeout as well as a total one, for both event subscriptions and crawl output streams. AWS ships no default operation timeout and tells users to set one. Goodfetch's defaults should be finite (ADR 0028 built-in defaults).
18. **Anthropic's long-request guard is a precedent for refusing a call that cannot finish within its own default deadline,** and telling the user what to use instead (stream, run in background). It applies to an `act` or `extract` with a large budget, or a crawl awaited synchronously.

### [#468 Concurrency model per language](https://github.com/goodfetch/automan/issues/468)

19. **Python: two classes with the same method names, as Azure (`.aio`), Stainless (`OpenAI`/`AsyncOpenAI`) and httpx do.**
    - The sync class must work inside a running event loop (Jupyter, FastAPI), which a facade over `asyncio.run` cannot.
    - ADR 0038's dedicated event-loop thread is the right answer. #468 should test it in Jupyter explicitly.
    - The async class should run on asyncio and ideally AnyIO/Trio, as httpx does.
20. **Python minimum 3.11:** 3.10 reached end of life on 2026-10-01, and 3.11 brings `TaskGroup`, `asyncio.timeout`, `ExceptionGroup` and `Unpack` natively (§4.1). This is the floor; ADR 0024's py3-none wheel still covers newer versions.
21. **Rust: async only, `Send + Sync` handles that are cheap to clone (`Arc` inside),** with `assert_send`/`assert_sync` tests (C-SEND-SYNC).
    - Tokio is acceptable as a hard dependency, because the crate links the engine, which is Tokio (ADR 0038).
    - A blocking wrapper is the second most-requested AWS Rust feature (pain 3; aws-sdk-rust#505, 104 reactions). #468 should decide whether to offer one or point to `block_on`.
22. **TypeScript: async only.** Iterate streams with `for await`, and have breaking out of a loop cancel the underlying subscription (Stainless's `Stream` aborts its request on `break`).

### [#469 Typed data: schemas, Records and payloads](https://github.com/goodfetch/automan/issues/469)

23. **Results tolerate the unknown; inputs are strict.**
    - Stainless keeps unknown fields (`extra="allow"`), does not validate responses by default, and types enums as `Literal` so that new values pass.
    - Azure gives Rust enums an `UnknownValue` variant.
    - Strict response validation was a top openai-python pain (pain 9).
    - For Goodfetch, this governs how an older SDK reads a newer engine's results within one protocol `major.minor` (ADR 0024 handshake), even though the SDK and engine ship together (ADR 0030).
24. **Accept user schemas through standards, not through one library's internals.**
    - TypeScript: accept any Standard Schema and get JSON Schema through Standard JSON Schema (Zod 4.2+, ArkType, Valibot). A Zod major then never breaks Goodfetch: the Zod 4 issues were openai-node's most-reacted (pain 2).
    - Python: Pydantic models and `TypeAdapter`-compatible types (`model_json_schema()` emits draft 2020-12).
    - Rust: `serde` plus `schemars`, which `goodfetch-protocol` already uses (ADR 0038).
25. **Rust models: avoid "everything is `Option<T>`"** (pain 5, aws-sdk-rust#536). Generate required fields as plain types, and keep `Option` for truly optional ones.
    - Results: `#[non_exhaustive]` structs (Azure, response-only).
    - Parameters: avoid `#[non_exhaustive]` if callers build them with `..Default::default()`, or provide builders instead (§3.2, §3.7).

### [#470 Settings in code](https://github.com/goodfetch/automan/issues/470)

26. **Three states for every optional setting: absent, explicitly null, value.**
    - Every leader needed this: Stainless `NotGiven`/`Omit`, httpx `USE_CLIENT_DEFAULT`, TypeScript `exactOptionalPropertyTypes`.
    - ADR 0028's layers need exactly these meanings: absent inherits from the lower layer, null clears or disables where the setting allows it, a value overrides.
    - Azure's "do not hard-code the server's defaults" is the matching rule: the SDK sends only what the user set and lets the engine resolve the layers. SDK-side defaults would silently bypass an Operator policy.
27. **Do not validate engine-side constraints in the SDK** (Azure `general-params-server-validation`). The engine owns the settings schema and the lock and cap rules (ADR 0028). The SDK validates only what it alone knows, such as a closed handle.
28. **Shape per language:**
    - TypeScript: a trailing options object.
    - Python: keyword-only arguments, typed with `Unpack[TypedDict]` where they are many. Azure forbids options objects in Python.
    - Rust: one choice among an options struct with `Default`, a compile-time-checked builder (bon) or AWS-style builders. Azure's rule to always take `Option<Options>` as the last parameter keeps signatures stable as settings are added.

    The settings schema is generated, so all three can be produced from it.
29. **Configuration sources:**
    - Per-instance configuration must be able to opt out of every global source: environment variables, files and Operator policy where allowed (Azure `general-config-optout`, `general-config-for-different-clients`).
    - Two instances in one process must be able to differ.
    - Stainless's environment defaults for keys and base URLs are convenient, but Azure's Rust rule against reading the environment in the library is a reminder that environment reading belongs to one documented layer.

### [#471 Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471)

30. **Same verb for the same operation in every language and surface** (Azure `general-client-standardize-verbs`). Use reserved prefixes with fixed meanings, chosen once in #471 and enforced by codegen:
    - `list_` for paged results;
    - a long-operation prefix (`begin_` or `start_`) or an awaitable handle;
    - `get_…`, or for Rust no `get_` on getters (C-GETTER).
31. **Collections:**
    - Iterate the elements themselves, with pages on request and a serializable continuation (Azure pagination rules).
    - TypeScript: `AsyncIterable` plus `.byPage()`. Python: iterator plus `by_page()`. Rust: a `Stream`-like type plus `into_pages()`.
    - For Rust, choose between implementing `futures::Stream` (Azure, kube, async-openai) and an own type with `next()` (AWS, for C-STABLE). `futures` is still 0.3 and `AsyncIterator` is not coming in 2026.
32. **Long operations as awaitable handles:**
    - `PollerLike extends Promise` (TypeScript), `result()` (Python), `Poller: IntoFuture + Stream` (Rust).
    - Each handle exposes status, progress and a serialized resume token. A Run handle in Foxhound and a long `agent` call in Collie are such operations.
33. **Fluent builders in Rust:** `IntoFuture` lets `page.click(sel).timeout(d).await` drop `.send()` (std docs; twilight, bon). AWS has not adopted it, and there is no measurable demand there. Decide once, apply everywhere, and note that a missing required field is a runtime error with AWS-style builders but a compile error with bon.
34. **One span per public method, with names derived from the catalog name** (Azure tracing rules). Logs never go to stdout (already ADR 0038). Treat log output as a contract that may only grow in minors (Azure compatibility), because agents and tests read it.
35. **The catalog's description is the docstring** (ADR 0038). Keep it short and structured: what it does, when to use it, when not, parameters, caveats, a minimal example with the canonical import. Over-long descriptions regressed agents (§5.1).

### [#484 Agent-writability check](https://github.com/goodfetch/automan/issues/484)

36. **Run the check as an eval with held-out tasks, behaviour-based assertions and three conditions:** no docs, an `AGENTS.md`-style compressed docs index, and full docs. This follows Anthropic's tool guide and Vercel's method, where the index scored 100% against 53% for none.
37. **Measure first-time correctness by type checker and test.** In model-written TypeScript, 94% of compilation errors were type errors. Run `tsc --strict`, Pyright or mypy on `py.typed` stubs, and `cargo check` on every agent attempt. Count the errors before any feedback, then the turns to green.
38. **Look for what the evidence says goes wrong:**
    - names that look like Playwright, httpx or Crawlee but behave differently;
    - similar method names;
    - required positional parameters versus options;
    - units of time;
    - stale idioms in examples. Deprecated usage reaches 70–90% when nearby code is outdated, so every example must be current; pre-release Goodfetch keeps no aliases, which matches.
39. **Ship the agent surfaces with the SDK:**
    - `llms.txt` with Markdown twins;
    - a pasteable `AGENTS.md` index snippet;
    - a `context7.json` with rules;
    - docs inside the packages (TSDoc in `.d.ts`, docstrings with `py.typed`, rustdoc).
    - Claim the package names agents will guess (5–22% package hallucination).

    These go to the docs area, but #484 should test against them.

### Tensions worth raising

- **Deprecation aliases after 1.0 versus agent evidence.**
  - ADR 0030 and the consumer API research favour aliases with warnings for a cycle.
  - The agent studies show stale usage follows context. They also show that a type-level `@deprecated` (PEP 702, TSDoc, Rust `#[deprecated]`) is what lets the checker flag it.
  - Reconcile the two: aliases must always carry the type-level marker, never only a runtime warning.
- **Error class per code (ADR 0038) versus Azure's "new type only for a different remedy".** Both can hold if classes group by remedy and codes stay fine-grained. #466 should state which one the user catches.
- **`#[non_exhaustive]` on shared protocol types.** The Rust SDK reuses `goodfetch-protocol` types (ADR 0038). Marking result types non-exhaustive protects older SDKs but forbids struct literals in users' tests, for example when building a fake result. A `test-util` constructor feature (AWS's approach) would answer it.

## 9. Unverified

**Azure**

- The page dates are the source files' last commits. Fragments included in a page (such as `refs.md`) may have changed later.
- No public way to resume a Rust `Poller` was found in azure_core 1.1.0 (`rust-lro-restart`). It may exist in generated crates or in 1.2.0-beta.1.
- `@azure/logger` appears to sit on top of `debug`; its implementation was not read.
- No Rust guideline names `IntoFuture`; it appears only in azure_core's code.
- The rationale in Azure/typespec-rust#809 was not read.
- Only the title and opening of azure-sdk-for-js#20380 were read.

**Stainless**

- The per-language status beyond the docs index (Rust and Swift "Planned") and the `Literal` quote come from a summarizer's reading of Stainless pages, not a raw read.
- Neither Anthropic README has a versioning section any more. The platform docs were not read.
- It is not established that neither Python SDK has a per-request cancellation mechanism beyond closing streams or tasks; open issue #2643 suggests none exists.
- That no Stainless SDK sends an idempotency header was checked only in the two Python clients, plus the Node hook and the Anthropic TypeScript client by text search.
- Whether Anthropic TypeScript caps `retry-after` was not checked.
- The bodies of openai-node#1576 and anthropic-sdk-python#1919 were not fetched; their state comes from search results.

**Rust**

- That reqwest, sqlx, kube and octocrab have no `IntoFuture` impl rests on GitHub code search of the default branch only.
- typed-builder's release date differs between crates.io (2025-11-19) and docs.rs (2026-08-31).
- bon's comparison page is written by bon's authors and was read through a summarizer.
- Return-type notation stabilizing "late this year" is a stated intent in a project goal, not a commitment.

**Python and TypeScript**

- Where the "Added in 3.11" note sits for `asyncio.timeout()`.
- BCD's Chrome version for `AbortSignal.timeout`.
- The `fetch` abort error name with no explicit reason.
- Whether Playwright for Python supports `async with` on `Page` and `Browser`.
- The exact date of the conditional Stage 4 decision.
- When tRPC, TanStack Form and Hono each added Standard Schema.
- Pydantic's performance claims.

**Coding agents**

- **No primary study of overloads, builders or fluent chaining** on agent correctness was found. Lesson 33 rests on design reasoning, not evidence.
- No evidence was found of agents confusing Pydantic v1/v2 or Zod 3/4.
- No dedicated Speakeasy or Fern guidance on agent-ready SDKs was found.
- Stainless's posts on agent experience (2026-03-23/24) were seen by title only.
- Bibliographic details:
  - AGENTS.md's exact launch day and Context7's public launch date;
  - CodeUpdateArena's fine-tuning numbers and VersiCode's migration scores, which appear only in figures;
  - the venue of the type-constrained generation paper (PLDI 2025, inferred from its DOI).
- No direct comparison of TypeScript and JavaScript on agent correctness was found; the 94% type-error figure supports it only indirectly.
- The size of Next.js's `llms-full.txt` comes from a commenter's memory.
