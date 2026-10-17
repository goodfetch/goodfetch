# Each request may carry its caller's Trace context across Goodfetch's own links, and the engine's spans stay Experimental

Each JSON-RPC request Goodfetch sends or receives may carry a W3C Trace context in `params._meta`: `traceparent` and `tracestate`, nothing else. It crosses every link between Goodfetch's own processes, and never reaches a target site or any other third party. A process uses it only while its own OTLP export is on. The dispatcher opens each request's span from its catalog entry, so no handler holds request-level tracing code. Span names and fields are Experimental, outside semver, until the OpenTelemetry conventions they follow are Stable.

Without a Trace context, an OTLP export showed the engine's spans as roots that no caller's trace could join: a call from a program into the engine and on to a Browser host was three unrelated traces. The envelope field is free to add before the first publish, and a contract change after it (ADRs 0030 and 0066).

Decided with the owner in [Trace context has no place in the protocol envelope, and span names are not a stated contract](https://github.com/goodfetch/goodfetch/issues/573), which follows gaps 6 and 7 of the [competitor codebase design research](https://github.com/goodfetch/goodfetch/pull/555). This decision builds on [Adopt runtime and state decisions](https://github.com/goodfetch/goodfetch/issues/70) item 9: `tracing` everywhere, opt-in OTLP export and GenAI conventions for model calls.

## Evidence

- **Stagehand v4** carries `traceparent` and `tracestate` in its JSON-RPC envelope and opens one server span per request, which the research found done well. It also opens a span for every log line, which this ADR avoids.
- **Playwright** reads per-method metadata from its generated protocol spec in its dispatcher and its tracing, so handlers hold no tracing code.
- **MCP's [SEP-414](https://modelcontextprotocol.io/seps/414-request-meta)** (Final) carries `traceparent`, `tracestate` and `baggage` in a request's `params._meta`, un-prefixed, for compatibility with OpenTelemetry. Goodfetch's MCP server receives them there.
- **OpenTelemetry's [RPC span conventions](https://opentelemetry.io/docs/specs/semconv/rpc/rpc-spans/)** are a Release Candidate. A span is named `{rpc.method}`, and a server receiving a request opens a `SERVER` span. Their `jsonrpc` system is still Development, and `rpc.system` has just been renamed `rpc.system.name`.
- **OpenTelemetry's [telemetry stability](https://opentelemetry.io/docs/specs/otel/telemetry-stability/)** lets an Unstable instrumentation change span names and attributes freely. A Stable one may only add, and must not change what it emits outside the semantic conventions.
- **[W3C Trace Context](https://www.w3.org/TR/trace-context/)** has a receiver ignore a `traceparent` it cannot parse and restart the trace, drop `tracestate` when `traceparent` is missing or invalid, and keep personal data out of `tracestate`.

## Decision

### The envelope

- **Every request's params may hold `_meta`,** a closed type with two optional fields, `traceparent` and `tracestate`, in W3C Trace Context's formats. `xtask codegen` adds the one shared type to every request's params, and an unknown key in it is refused with `invalid_params`, as for every input (ADR 0082). The leading underscore keeps it apart from every snake_case name (ADR 0099). The schema checks only that each value is a string within W3C's length bound.
- **W3C Baggage is not carried.** It holds application-defined key-value pairs, which may be personal data, and Goodfetch has no use for it.
- **Responses and notifications carry none:** events, `$/cancel` and `$/log` included.
- **No SDK exposes `_meta`** in its public API. Each runtime fills it in.

### The links

- **It crosses each of Goodfetch's own links, in both directions:**
  - a Client's requests to an engine;
  - an engine's requests to SDK handlers (ADR 0104);
  - an engine's requests to a Browser host (ADR 0096);
  - a call forwarded over the peer link of a Daemon group (ADR 0055);
  - Goodfetch Cloud's gateway, which passes it on unchanged;
  - the plugin protocol's requests to a process Plugin (ADR 0033);
  - the MCP server's calls to the engine, taking the MCP client's `_meta.traceparent` and `tracestate`;
  - the HTTP listener (ADR 0027), which also reads an HTTP `traceparent` header when `_meta` has none, since workflow tools inject one.
- **It never reaches a third party.** No trace header goes to a target site, through a Fetch tier or from a browser, where it would mark the request as automated, nor to a Model provider, Sink, webhook, Notification or Secret provider (ADR 0043). The engine's HTTP client injects no trace header.

### What a receiver does

- **With no `traceparent`,** the request's span is a new root.
- **A `traceparent` it cannot parse** is ignored: the span starts a new trace and `tracestate` is dropped, as W3C says. A `tracestate` without a `traceparent`, or one that is invalid, is dropped, and a valid `traceparent` is kept.
- **None of these fails the call.** Each leaves one `debug` event.
- **A process uses Trace context only while its own OTLP export is on.** With export off, it ignores what it receives and sends none. Each process decides for itself: a Browser host without export ignores the context its engine sends.

### The SDKs

- **An SDK passes on its host program's ambient OpenTelemetry context:**
  - TypeScript, from `@opentelemetry/api`'s active context when that package is installed, an optional peer dependency;
  - Python, from `opentelemetry-api` when it can be imported, with no required dependency;
  - Rust, behind an `opentelemetry` Cargo feature of the SDK crate, from the current `tracing` span through `tracing-opentelemetry`. A heavy optional dependency is what ADRs 0038 and 0087 allow a published crate's features for.
- **An SDK handler runs under the context the engine sent,** made current around the user's code, so its spans nest under the engine's.
- **An SDK always sends the context its host program has,** since it cannot know the engine's export setting.
- **SDKs open no spans of their own in V1,** so the engine's request span is the user's span's direct child. Adding SDK spans later breaks nothing.

### Spans

- **The dispatcher opens one `SERVER` span per request it receives,** from the request's catalog entry, and the RPC layer opens one `CLIENT` span per request the process sends. Both follow OpenTelemetry's RPC and JSON-RPC conventions:
  - the span is named by the operation's canonical name (`run.start`, `page.goto`);
  - its fields are `rpc.system.name` (`jsonrpc`), `rpc.method`, `jsonrpc.request.id`, the reply's Error code, and the glossary IDs found in the params (`run_id`, `session_id`).
- **Handlers hold no request-span code.** They open their own child spans for runs, pages and CDP commands (`STYLE.md`).
- **A run is its own trace.** Its span is a new root, linked by an OpenTelemetry span link to the request span of the call that started it. A resumed run starts a new root, linked to the request that resumed it. Run state records nothing about traces.
- **Span fields hold bounded values only:** IDs, the operation name, Error codes and values from bounded sets such as tier and browser kind. A URL is recorded as its origin. Params, page content, extracted data and Secrets never reach a span.

### Stability

- **Span names, span fields and the engine's targets in an export are Experimental,** outside semver like log text (ADRs 0030 and 0079). A change may land in a minor and is listed in its release notes.
- **A span or field may be promoted to Stable** once the OpenTelemetry convention it follows is Stable.
- **What is already under semver stays there:** operation names, which become request spans' names, and the Rust SDK's own `goodfetch::…` targets (ADR 0078).

## Considered Options

- **Trace context in `hello`, once per connection:** rejected. A Daemon serves several callers, and one SDK connection makes calls that belong to different traces.
- **A top-level envelope member** beside `jsonrpc`, `id`, `method` and `params`: rejected. JSON-RPC 2.0 defines no such member, strict JSON-RPC parsers reject it, and Goodfetch's MCP server already receives the same fields in `params._meta`.
- **No Trace context, with traces correlated by `run_id`:** rejected. Every engine span would stay a root that no caller's trace can join.
- **Carrying W3C Baggage:** rejected. Its values are application data, which may be personal, and Goodfetch reads none of them.
- **Trace context used whatever the export setting,** so that logs and `$/log` events carry trace and span IDs: rejected. It keeps an OpenTelemetry context in every process at all times, for a log correlation no user asked for.
- **Span names and fields Stable, declared in `goodfetch-protocol` as metric names are (ADR 0050):** rejected for V1. Metrics are what dashboards and alerts read, and they are already versioned. The conventions spans follow are still moving, and promoting a span later only adds, where retracting a promise breaks. A split, with only the request span Stable, was rejected for the same reason.
- **An `#[instrument]` request span on each handler:** rejected. One handler forgetting it, or naming it differently, would break the rule that every request has its span, which the dispatcher keeps in one place.
- **A run's span as a child of the `run.start` request:** rejected. Its trace would last hours under a parent that has already ended, and the chain would break when the run resumes on another machine. OpenTelemetry uses links for work that outlives the request that starts it.
- **Client spans in each SDK:** deferred. SDKs would need OpenTelemetry's API as a dependency rather than an optional one, and the engine's span already sits under the caller's.

## Consequences

- **Amends ADR 0038.** Every request's params may hold `_meta`, and the RPC layer that converts errors for the wire also opens each request's span and reads and writes its Trace context. A note sits at its Errors bullet.
- **Amends ADR 0030.** Semver does not cover span names, span fields or the engine's targets in an export. A note sits at its list.
- **Amends ADR 0050.** Metric names stay a versioned contract and spans do not. A note sits at its Metrics list.
- **Amends ADR 0078.** The Rust SDK's targets stay under semver, and the same names in an engine's export do not. A note sits at its Every SDK bullet.
- **`STYLE.md`'s tracing rules** say that the dispatcher opens request spans and what a span field may hold. **`TESTING.md`'s Secret canary search** covers the OTLP export, read through an in-process OTLP collector.
- **The Simulation** draws trace and span IDs from its seeded randomness, so its traces replay (ADR 0060).
- **ADR 0037.** Row RT-18 and pain row 19 (`dx-observability`) cite this ADR. Area 3, Engine runtime, builds the envelope, the receiver's rules and the spans. Area 11, Client surfaces, builds what each SDK sends and the context an SDK handler runs under.
- **Glossary.** Adds **Trace context**, and **Experimental** also names metrics and spans.
