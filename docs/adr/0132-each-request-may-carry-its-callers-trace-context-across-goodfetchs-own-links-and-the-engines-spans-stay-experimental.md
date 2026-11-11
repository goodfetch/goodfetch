# Each request may carry its caller's Trace context across Goodfetch's own links, and the engine's spans stay Experimental

Each JSON-RPC request of Goodfetch's protocols may carry a W3C Trace context in `params._meta`: `traceparent` and `tracestate`, nothing else. It crosses every link of those protocols, and Goodfetch's own code never sends it to a target site or any other destination. An engine or a Browser host uses it only while its own OTLP export is on. The dispatcher opens each request's span from its catalog entry, so no handler holds request-level tracing code. Span names and fields are Experimental, outside semver, until an ADR promotes them.

Without a Trace context, an OTLP export showed the engine's spans as roots that no caller's OpenTelemetry trace could join: a call from a program into the engine and on to a Browser host was three unrelated traces. The envelope field is free to add before the first publish, and a contract change after it (ADRs 0030 and 0066).

Decided with the owner in [Trace context has no place in the protocol envelope, and span names are not a stated contract](https://github.com/goodfetch/goodfetch/issues/573), which follows gaps 6 and 7 of the [competitor codebase design research](https://github.com/goodfetch/goodfetch/pull/555). This decision builds on [Adopt runtime and state decisions](https://github.com/goodfetch/goodfetch/issues/70) item 9: `tracing` everywhere, opt-in OTLP export and GenAI conventions for model calls.

## Evidence

- **Stagehand v4** carries `traceparent` and `tracestate` in its JSON-RPC envelope and opens one server span per request, which the research found done well. It also opens a span for every log line, which this ADR avoids.
- **Playwright** reads per-method metadata from its generated protocol spec in its dispatcher and its tracing, so handlers hold no tracing code.
- **MCP's [SEP-414](https://modelcontextprotocol.io/seps/414-request-meta)** (Final) carries `traceparent`, `tracestate` and `baggage` in a request's `params._meta`, un-prefixed, for compatibility with OpenTelemetry. Goodfetch's MCP server receives them there.
- **OpenTelemetry's [RPC span conventions](https://opentelemetry.io/docs/specs/semconv/rpc/rpc-spans/)** are a Release Candidate. A span is named `{rpc.method}`, and a server receiving a request opens a `SERVER` span. Their `jsonrpc` system is still Development, and `rpc.system` has just been renamed `rpc.system.name`.
- **OpenTelemetry's [telemetry stability](https://opentelemetry.io/docs/specs/otel/telemetry-stability/)** lets an Unstable instrumentation change span names and attributes freely. A Stable one may only add, and must not change what it emits outside the semantic conventions.
- **[W3C Trace Context](https://www.w3.org/TR/trace-context/)** has a receiver ignore a `traceparent` it cannot parse and restart the trace, drop `tracestate` when `traceparent` is missing or invalid, and keep personal data out of `tracestate`. A version-00 `traceparent` is 55 characters, and a vendor should pass on at least 512 characters of `tracestate`.

## Decision

### The envelope

- **Every request's params may hold `_meta`,** a closed type with two optional string fields, `traceparent` and `tracestate`, in W3C Trace Context's formats. It belongs to the envelope, not to any operation: it is declared once in `goodfetch-protocol`, `xtask codegen` adds it to every request's params on the wire, and no generator gives it a member, since no SDK exposes it in its public API and each runtime fills it in. Its leading underscore keeps it apart from every catalog name (ADR 0099).
- **An unknown key in `_meta` is refused,** as ADR 0077's G3 refuses any unknown input. An SDK refuses a `_meta` a program writes into a raw `call`'s params, since the runtime alone writes it.
- **W3C Baggage is not carried.** It holds application-defined key-value pairs, which may be personal data, and Goodfetch has no use for it.
- **Responses and notifications carry none:** events, `$/cancel` and log events included.

### The links

- **It crosses each link of Goodfetch's protocols, in both directions:**
  - a Client's requests to an engine;
  - an engine's requests to SDK handlers (ADR 0104);
  - an engine's requests to a Browser host (ADR 0096);
  - a call forwarded over the peer link of a Daemon group (ADR 0055);
  - Goodfetch Cloud's gateway, which passes it on unchanged;
  - the plugin protocol's requests to a process Plugin the user installed (ADR 0033);
  - the MCP server's calls to the engine, taking the MCP host's `_meta.traceparent` and `_meta.tracestate`;
  - the HTTP listener (ADR 0027), which also reads the HTTP `traceparent` and `tracestate` headers when `_meta` has none, since workflow tools inject them.
- **Goodfetch's own code never sends it anywhere else.** No trace header goes to a target site, through a Fetch tier or from a browser, where it would mark the request as automated, nor to the endpoints of a Model provider, Sink, webhook, Notification channel or Secret provider (ADR 0043). The engine's HTTP client injects no trace header. A process Plugin or SDK handler of one of those kinds is a link above: it receives the Trace context of the engine's request to it like any other, and what it sends on is its own.

### What a receiver does

- **With no `traceparent`,** the request's span is a new root.
- **A value it cannot use is dropped, never refused:** a `traceparent` that does not parse, or is longer than 256 characters, starts a new OpenTelemetry trace and drops `tracestate`, as W3C says; a `tracestate` longer than 512 characters, invalid on its own, or sent without a valid `traceparent`, is dropped and a valid `traceparent` is kept. The schema checks only that each value is a string.
- **None of these fails the call.** Each leaves one `debug` event.
- **An engine or a Browser host uses Trace context only while its own OTLP export is on.** With export off, it ignores what it receives and sends none on. Each decides for itself: a Browser host without export ignores the Trace context its engine sends. Clients do not apply this rule: an SDK, the MCP server and the HTTP listener's callers send what their caller has, and the gateway passes it on unchanged, since none of them can know the receiving engine's export setting.

### The SDKs

- **An SDK passes on its host program's ambient OpenTelemetry context:**
  - TypeScript, from `@opentelemetry/api`'s active OpenTelemetry context when that package is installed, an optional peer dependency;
  - Python, from `opentelemetry-api` when it can be imported, with no required dependency, taken on the caller's thread by the sync API;
  - Rust, through a trait of `goodfetch-collie-client` that reads the current OpenTelemetry context, which `goodfetch-collie` and `goodfetch-foxhound` implement behind an `opentelemetry` Cargo feature through `tracing-opentelemetry`. The client crate depends on no OpenTelemetry crate, and the feature sits on the crates through which each Product's SDK embeds its engine, as ADRs 0038 and 0087 allow for a heavy optional dependency.
- **An SDK handler runs under the Trace context the engine sent,** made current around the user's code, so its spans nest under the engine's.
- **SDKs open no spans of their own in V1,** so the engine's request span is the user's span's direct child. Adding SDK spans later breaks nothing.

### Spans

- **In an engine and a Browser host, the RPC layer's dispatcher opens one `SERVER` span per request received,** from the request's catalog entry, **and the RPC layer one `CLIENT` span per request sent,** whose Trace context it writes into that request's `_meta`. Both follow OpenTelemetry's RPC and JSON-RPC conventions:
  - the span is named by the operation's canonical name (`run.start`, `page.goto`);
  - its fields are `rpc.system.name` (`jsonrpc`), `rpc.method` and `jsonrpc.request.id`, a failed reply's Error code as `error.type` and its JSON-RPC numeric code as `rpc.response.status_code`, and the glossary IDs found in the params (`run_id`, `session_id`).
- **Handlers hold no request-span code.** They open their own child spans for runs, pages and CDP commands (`STYLE.md`).
- **A run is its own OpenTelemetry trace.** Its span is a new root, linked by an OpenTelemetry span link to the request span of the call that started it. A resumed run starts a new root, linked to the request that resumed it. Run state records no trace or span ID.
- **Span fields hold IDs, the operation name, Error codes and values from bounded sets** such as tier and browser kind. A URL is recorded as its origin. Params, page content, extracted data, model prompts and completions, and Secrets never reach a span.

### Stability

- **Span names, span fields and the engine's targets in an export are Experimental from 1.0,** outside semver like log text (ADRs 0030 and 0079). The reference says so, nothing warns at run time, and a change may land in a minor and is listed in its release notes.
- **A later ADR may promote a span or field to Stable.** One that follows an OpenTelemetry convention is promoted no earlier than that convention is Stable. The run, page and CDP command spans follow none, so they stay Experimental until an ADR gives them a stated shape.
- **What is already under semver stays there:** operation names, which become request spans' names, and the Rust SDK's own `goodfetch::…` targets (ADR 0078).
  > Note (agent decision, accepted by the owner on 2026-10-22), 2026-10-22, [#257](https://github.com/goodfetch/goodfetch/issues/257): an engine process's OTLP export is on once the setting `otlp.traces_endpoint` (user configuration or Operator policy only) names an OTLP/HTTP collector's traces URL, such as `http://localhost:4318/v1/traces`, and off while it is unset, its default; the executable then exports every span, never the log events inside them, in protobuf from the OpenTelemetry SDK's batch processor, under the service name `goodfetch-<product>`, with the span's `target` but no source path or thread name. The handlers' own spans are `run` (`run_id`, a root of its own, at `info`), `page` (`page_id`, `run_id`, `operation`, and `session_id` or `url.origin` where they apply, at `debug`) and `cdp_command` (`cdp.method`, at `trace`). The setting's description in the catalog, from which the reference is generated, marks span names, span fields and targets Experimental. #1557 adds the request spans and `_meta` on this export.
  > Note (agent decision, on a guarantee, not read by the owner), 2026-10-22, [#1557](https://github.com/goodfetch/goodfetch/issues/1557): `_meta`'s type is `TraceContext` in `goodfetch-protocol`'s `jsonrpc` module; emitting the bundle adds it to each params type of an operation, a handler request, a Plugin method and `hello`, and writes it once as the bundle's `connection.meta`, so a params type is never another type's member (the Foxhound sample's `echo` now takes `SampleEchoed`); the SDK layers drop it from every params type and keep the `TraceContext` type for their runtimes. `hello` carries it too: its span opens only once a listener's token is checked, so a stranger's Trace context is never read. A `_meta` that is no `TraceContext` is refused with `invalid_params`. The request spans are `request` spans of the target `goodfetch::rpc::trace` at `debug`, which take the operation's name in the export through `otel.name`; `jsonrpc.request.id` is recorded only for a number id, since a text id is any text its sender chose; a peer's Error code is recorded only in an Error code's shape. Export is on for the RPC layer when an OpenTelemetry layer takes its spans, so a process without one reads no Trace context and sends none. Trace and span IDs come from an `Entropy` fork of their own (`EntropyIds`) in the executable as in tests, so a seed fixes them.

## Considered Options

- **Trace context in `hello`, once per connection:** rejected. A Daemon serves several callers, and one SDK connection makes calls that belong to different traces.
- **A top-level envelope member** beside `jsonrpc`, `id`, `method` and `params`: rejected. JSON-RPC 2.0 defines no such member, strict JSON-RPC parsers reject it, and Goodfetch's MCP server already receives the same fields in `params._meta`.
- **No Trace context, with traces correlated by `run_id`:** rejected. Every engine span would stay a root that no caller's trace can join.
- **Carrying W3C Baggage:** rejected. Its values are application data, which may be personal, and Goodfetch reads none of them.
- **Trace context used whatever the export setting,** so that logs and log events carry trace and span IDs: rejected. It keeps an OpenTelemetry context in every process at all times, for a log correlation no user asked for.
- **Refusing an over-long or malformed value with an error:** rejected. Tracing must never break a run, and W3C has a receiver restart the trace instead.
- **Span names and fields Stable, declared in `goodfetch-protocol` as metric names are (ADR 0050):** rejected for V1. Metrics are what dashboards and alerts read, and they are already versioned. The conventions spans follow are still moving, and promoting a span later only adds, where retracting a promise breaks. A split, with only the request span Stable, was rejected for the same reason.
- **An `#[instrument]` request span on each handler:** rejected. One handler forgetting it, or naming it differently, would break the rule that every request has its span, which the dispatcher keeps in one place.
- **A run's span as a child of the `run.start` request:** rejected. Its OpenTelemetry trace would last hours under a parent that has already ended, and the chain would break when the run resumes on another machine. OpenTelemetry uses links for work that outlives the request that starts it.
- **The Rust feature on `goodfetch-collie-client`:** rejected. ADR 0087 keeps Cargo features to the crates that embed an engine, and a trait keeps the client crate free of OpenTelemetry.
- **Client spans in each SDK:** deferred. SDKs would need OpenTelemetry's API as a dependency rather than an optional one, and the engine's span already sits under the caller's.

## Consequences

- **Amends ADR 0038.** Every request's params may hold `_meta`, and the RPC layer that converts errors for the wire also reads and writes each request's Trace context, its dispatcher opening each received request's span. A note sits at its Errors bullet.
- **Amends ADR 0030.** Semver does not cover span names, span fields or the engine's targets in an export, which are Experimental from 1.0, marked in the reference only and never warned about. Notes sit at its list of what semver does not cover and at its Experimental bullet.
- **Amends ADR 0033.** Every request the engine sends a process Plugin may carry `_meta`. A note sits at its decision.
- **Amends ADR 0050.** Metric names stay a versioned contract and spans do not. A note sits at its Stable or Experimental bullet.
- **Amends ADR 0078.** The Rust SDK's targets stay under semver, while the engine's targets in an export are Experimental. Each SDK passes on its host program's OpenTelemetry context and runs an SDK handler under the engine's Trace context. A note sits at its Every SDK bullet.
- **Amends ADR 0099.** `_meta` is an envelope field, not a catalog parameter: its underscore is outside the snake_case word rule, and no generator gives it a member. A note sits at its name rule.
- **Tests.** Through an in-process OTLP collector: the receiver rules and both span kinds in the engine's tests; propagation across each link in that link's tests and in the protocol conformance cases; and no `traceparent`, `tracestate` or `baggage` header in what the Proving ground, the Fake model, each Sink's container and an `httpmock` webhook endpoint receive (`TESTING.md`'s Trace context conformance cases). **`TESTING.md`'s Secret canary search** covers the OTLP export.
- **`STYLE.md`'s tracing rules** say that the dispatcher opens request spans and what a span field may hold.
- **The Simulation** draws trace and span IDs from its seeded randomness, so its OpenTelemetry traces replay (ADR 0060).
- **ADR 0037.** Row RT-18 and pain row 19 (`dx-observability`) cite this ADR. Area 3, Engine runtime, builds the envelope, the receiver's rules and the spans. Area 11, Client surfaces, builds what each SDK sends and the Trace context an SDK handler runs under.
- **Glossary.** Adds **Trace context**, and **Experimental** also names metrics and the engine's trace output.
