# Code style

How code is shaped in Goodfetch: the engine, its crates, the SDKs and the tools around them. Every rule comes with its reason, so that an exception can be judged against it. The hard-to-reverse rules (contracts, errors on the wire, cancellation, casing, concurrency, Cargo features) are decided in [ADR 0038](docs/adr/0038-contracts-from-rust-types-one-error-cancellation-and-concurrency-model.md); this file restates them only as far as code needs them. Rules a tool can check also live in the tool's configuration, which wins when the two disagree, and this file is then corrected.

Where this file is silent, the Rust section names its references and `AGENTS.md`'s Language skills lists the skills for each language; this file wins where they disagree.

## Design

The design vocabulary is the one of `codebase-design`: a **module** has an **interface** (everything a caller must know, not only its types), it sits at a **seam**, and it is **deep** when a lot of behaviour hides behind a small interface.

- **A crate's public API is its interface.** Everything else is `pub(crate)` or private, and the `unreachable_pub` lint warns on a `pub` that nothing outside can reach.
  _Why:_ what callers can't see, maintainers can change freely.
- **Concrete types first; a trait only where two adapters exist.** A test fake counts as the second adapter only when the dependency has no local stand-in: SQLite in a temporary file is a stand-in, so it gets no trait; a remote model provider has none, so a mock adapter is justified. Every external dependency is still injected, never reached through a global: as a value (the Goodfetch home, an address, the environment map) when a stand-in exists, and as a handle only for time (`Clock`) and random choices (`Entropy`), whose test versions are the second adapter (ADR 0059, rules in [`TESTING.md`](TESTING.md)). The Plugin kinds of ADR 0033 are real seams, since built-ins, process Plugins and SDK handlers all implement them.
  _Why:_ a trait with one implementation is indirection with no payoff, and it spreads generics and slows compiles.
- **What every implementation of a contract shares lives once, in a driver above them** ([ADR 0129](docs/adr/0129-what-every-implementation-of-a-contract-shares-lives-once-in-a-driver-above-them-and-each-implementation-fills-only-narrow-primitives.md)). Redirects, retries, cookies, the typed-failure enum and what each variant leads to, validation and timeouts sit in a concrete driver type; each implementation (a fetch tier, a Sink, a Model provider, a local browser or a Browser host) fills only a narrow trait of primitives, such as one `send` per hop. Each area spec names its drivers and primitives.
  _Why:_ copies along an implementation axis drift apart, even under one conformance suite.
- **Prefer types over generics, and generics over `dyn`.** When `dyn` is needed, hide it in a newtype of our own (`struct Transport(Arc<dyn TransportImpl>)`) instead of showing `Arc<dyn …>` in an interface.
  _Why:_ the interface stays stable if the dispatch strategy changes.
- **State with one owner is an actor.** A task owns it and receives messages through a channel; no `Arc<Mutex<…>>` around a connection, a CDP session or a run's writer.
  _Why:_ one owner removes lock ordering, lock-held-across-await bugs and shared mutation.
- **Each module has one concern, named in its `//!` comment** ([ADR 0126](docs/adr/0126-each-crate-declares-which-of-its-modules-may-import-which-each-module-names-one-concern-and-a-file-past-1000-lines-states-why.md)). A role that needs "and" to join two jobs is two modules.
  _Why:_ every central class in the competitor research grew by taking the next feature where it was easiest to add.
- **Tests go through the interface.** If a test needs to reach past it, the module is the wrong shape.
  _Why:_ such tests survive internal refactors, and they test what callers rely on.

## Names

- **Identifiers use the terms of [`GLOSSARY.md`](GLOSSARY.md), never its `_Avoid_` terms.** It is `Session`, not `Login` or `Profile`; `Sink`, not `Output` or `Connector`. A concept the code needs that has no entry gets one in the glossary first.
  _Why:_ specs, docs, code and conversations then use one language.
- **Full words, no abbreviations** except those everyone writes (`id`, `url`, `http`, `cdp`).
- **Rust naming follows the Rust API Guidelines:** casing, `as_`/`to_`/`into_` for conversions by cost and ownership, no `get_` prefix on getters, `iter`/`iter_mut`/`into_iter`.
- **Names in the SDK API, the CLI and MCP come from the catalog's canonical words by one rule per language** ([ADR 0099](docs/adr/0099-every-surface-spells-each-catalog-name-by-its-own-rule-shapes-come-from-declared-kinds-and-call-is-the-way-down.md)): initialisms from its closed list are capitals in TypeScript names and Python class names (`waitForURL`, `CDPSession`), lowercase where they open a camelCase name (`url()`), and one word in Rust (`CdpSession`); a keyword is escaped, never renamed (`continue_` in Python, `r#continue` in Rust); no name keeps a second spelling beyond a deprecation's window, except a leader's tombstone a ticket chose (`waitForTimeout`).
  _Why:_ every surface then names an entry the same way, and a reader or a coding agent applies one rule without exceptions.

## Rust

References: the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html) and Microsoft's [Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/) (as one file for agents: [`all.txt`](https://media.githubusercontent.com/media/microsoft/rust-guidelines/19723b30aadf1854fef6a56c2da2395dc03255c1/src/agents/all.txt)).

### Workspace and crates

- **The root `Cargo.toml` is a virtual manifest.** Members live under `crates/`, `crates/foxhound/` (every crate only Foxhound links, ADR 0087) and `crates/foxhound/sinks/` (the sink processes, from the first the Sinks area adds), plus `proving-ground/` (ADR 0032) and an `xtask/` folder at the root for `xtask codegen` and `xtask release`. Each folder name equals its crate name, and internal crates are named `goodfetch-<glossary term>`, apart from the test-only `goodfetch-test-support` and `goodfetch-test-client` (a test's Client). A crate takes a Product's name only when it is that Product's executable crate or a crate the Product publishes for its users: `goodfetch-collie` or `goodfetch-foxhound` for the crate published as its Rust SDK, `goodfetch-<product>-<part>` for any other (ADR 0090). Siblings and shared dependencies are referenced with `.workspace = true`.
- **A concern becomes its own crate only for one of these reasons:**
  1. it is a separate executable (each Product's executable, a sink process, the sandbox helper, `xtask`, the Proving ground);
  2. the compiler must enforce a dependency direction (nothing outside `crates/foxhound/` but `xtask` depends on a crate inside it, so no Collie crate depends on a Foxhound crate (ADRs 0084, 0087); `goodfetch-protocol` never depends on the engine; the Proving ground never depends on engine crates; `goodfetch-conformance` depends only on `goodfetch-protocol`; `goodfetch-test-support` depends on no Goodfetch crate and is only ever a dev-dependency (ADR 0062); `goodfetch-test-client` depends on no Goodfetch crate but `goodfetch-protocol` and `goodfetch-test-support`, is only ever a dev-dependency and is never taken by `goodfetch-protocol` or the Proving ground (ADR 0062); a test adapter crate (`goodfetch-simulation`, `goodfetch-scripted-model`) is only ever a dev-dependency and is never taken by a unit test of a crate it depends on (ADR 0125); a Private component's interface with its no-op implementation);
  3. it is heavy and rarely changes, so isolating it speeds up incremental builds (generated CDP bindings);
  4. it is a Plugin-kind seam whose built-in implementation pulls dependencies nothing else needs.

  Otherwise it is a module. Each area's spec names its crates by these reasons.
  _Why:_ crates build in parallel and enforce directions, but each one adds visibility plumbing; a stated reason keeps them meaningful.
- **Each Product's executable crate is its composition root** (ADR 0087). Each domain crate exports one value listing what it adds to the engine (operation handlers, settings keys, run-state tables, Plugin kinds and Hook stages, Usage kinds and Limit scopes, the Notifications it can send, gates in admission), and the executable crate passes the values it links to the engine at start; Foxhound's starts from Collie's. Nothing registers itself by being linked, and code outside `crates/foxhound/` and `xtask` never names a Foxhound type, nor `goodfetch-protocol`'s `foxhound` module, which `xtask dependency-direction` checks (ADR 0099): Foxhound fills interfaces the engine defines.
  _Why:_ what a Product contains is then a list in its `main`, a test composes exactly what it needs, and Collie's code compiles without Foxhound's.
- **Module files are `foo.rs` beside a `foo/` folder, never `mod.rs`.** `lib.rs` only declares modules and re-exports.
  _Why:_ editor tabs full of `mod.rs` are unreadable, and a short `lib.rs` shows the crate's shape at a glance.
- **Each crate and each module opens with a `//!` comment stating its role.**
- **Each crate's `modules.toml` lists which of its top-level modules may import which** (ADR 0126). An `xtask` check in the Quick suite fails on any other import (type-only imports and a sibling's item imported through the root's re-export included, `#[cfg(test)]` code excepted), on a cycle, on a name that is no module of the crate, on a module the file does not list, and on a root that re-exports a module's items with a glob.
  _Why:_ Rust's visibility does not stop one sibling module from using another, and import rules held in the research only where a tool checked them.
- **A Rust file over 1,000 lines states why it is still one concern**, in a `Size:` line of its `//!` header, or the same `xtask` check fails. Every line of a file under `src/` counts, except `#[cfg(test)]` items (and files only they declare) and generated code. It is an alarm, not a cap.

### Errors

- **Each crate has its own typed errors, written with `thiserror`.** Public error types are `#[non_exhaustive]`.
- **`anyhow` only in `main`, `xtask` and tests.**
- **Every failure a Client can see maps to an Error code** from the list in `goodfetch-protocol` (snake_case string, retryable flag, JSON-RPC class). Only the RPC layer converts errors for the wire.
  _Why:_ Error codes are under semver (ADR 0030) and every SDK generates a class per code, so a code is matched on, never a message.
- **A failure from a party Goodfetch did not write becomes a typed error in one translation point per boundary** ([ADR 0127](docs/adr/0127-a-failure-from-a-party-goodfetch-did-not-write-becomes-a-typed-error-in-one-module-per-boundary-and-no-other-code-reads-its-text.md)): such as the CDP client, the transport primitive of each fetch tier that sends HTTP itself, the egress proxy where it forwards to an upstream proxy, each Model provider adapter and each built-in Sink's client; under a driver (ADR 0129) each maps into the enum the driver defines. The party's message is a private, redacted field with no string accessor, and only the translation point matches on it; an unknown failure becomes its `Unrecognized` variant, logged at `warn`.
  _Why:_ text matched wherever it is needed breaks in every place at once when the party rewords it.
- **No `unwrap()` outside tests.** `expect("invariant: …")` states an invariant the type system can't express; anything input can trigger returns an error.

### Async and concurrency

- **Tokio is the runtime, and concurrency is structured** (ADR 0038): child cancellation tokens from the engine process (an Attached engine or the Daemon) to each run, Session and page; each owner waits for its own tasks before it stops.
- **No task starts outside an owner's tracker.** Spawn through it (`TaskTracker::spawn`, `TaskTracker::spawn_blocking`); a Clippy `disallowed-methods` rule bans `tokio::spawn`, `tokio::task::spawn_blocking`, `tokio::task::spawn_local`, `Handle::spawn`, `Handle::spawn_blocking`, `Runtime::spawn`, `Runtime::spawn_blocking`, `LocalSet::spawn_local`, `std::thread::spawn`, `std::thread::Builder::spawn` and `std::thread::scope` outside the module that implements this (`tasks`). A tracked task may own a `JoinSet` for a fan-out whose results it awaits before it returns.
  _Why:_ an orphan task outlives its cancelled run and keeps writing.
- **An owner's parent is told of a task's panic, and ends the owner through an end that already exists** ([ADR 0148](docs/adr/0148-a-panic-ends-what-owned-its-task-the-engine-stops-only-for-a-part-every-run-shares-and-nothing-is-restarted-in-place.md)). The `tasks` module catches the panic of a task, an actor or blocking work whose caller has gone where it runs, logs it at `error` with the owner's kind and tells its parent through `TaskOwner::panicked`; the parent then ends the owner, as a Connection whose own task panicked ends as when its Client goes away. A task the parent starts on the parent's own owner, such as the one that serves a Connection, is watched where it is started instead. An owner of the engine's kind, a part every Run shares, tells its parent too, up to the engine's own owner, which the engine awaits, except of blocking work whose caller has gone, which was the caller's: it then stops, a Daemon draining and an Attached engine stopping its Runs, and its process exits 1. Work for one Run therefore starts on an owner of the Run's kind, never on the engine's or the Run registry's own. Until a Run's parent awaits `TaskOwner::panicked` (spec #2425's next tickets), the log line is a Run's panic's only trace. Nothing is restarted in place.
  _Why:_ stopping an owner waits for its tasks, so none of them can stop it, and a part restarted in place keeps the state a panic left half-changed.
- **Never block an async worker.** Blocking I/O and CPU-heavy work go through the `tasks` module's blocking policy, a value: the owner's tracked blocking threads, or a dedicated thread (an actor's, such as the run's SQLite writer) that `tasks` starts and its owner joins before it stops. In the Simulation the same policy runs blocking calls inline and a dedicated thread's body as a task on the host's runtime (ADR 0060); `std::thread::sleep`, blocking `std::process::Command` and blocking `std::fs` calls are disallowed in async crates, and so is `tokio::fs` everywhere, as it runs on the runtime's own blocking pool outside that policy, so a paused run under the inline policy would depend on the host's load (issue 2106): a file call runs as `std::fs` inside the owner's blocking work. A page's parsing and reading, a Page analysis's work, goes through `TaskOwner::analysis` instead: the CPU pool, whose permits, one per core, bound how many run at once on those threads (ADR 0138). One pass reads on the page's own task instead: the untouched readers' streaming pass over a crawl request's own document, which takes each decoded chunk as it arrives and holds its state across the body's awaits, its work per chunk a tokenizer step over that chunk (issue 2188).
- **Sockets and runtimes have one place each** (ADR 0060).
  - Connections the Simulation carries (stdio of an Attached engine, the Daemon's socket and named pipe, loopback TCP, the WebSocket listener, the private endpoint, the ops listener, the peer link, wake endpoints) are opened only in `goodfetch-engine`'s `transport` module, which takes the network as a value; `rpc` takes its connections from it. `disallowed-types` lists the socket types of `std::net`, `std::os::unix::net`, `tokio::net` and Tokio's named pipes, and `disallowed-methods` lists `tokio::io::stdin`, `tokio::io::stdout` and `std::io::stdin`. On the client side, `goodfetch-collie-client`'s `transport` module opens every socket and starts every Attached engine, over the network it is handed: the OS's or the engine's in-memory one (issue 1221). Fetch sockets behind the egress checks, the CDP connection, plugin-protocol pipes and third-party clients open their own, each module with an `#[expect]` naming ADR 0060. So does the test-only `goodfetch-scripted-engine`, which the Simulation never runs, with an `#[expect]` naming ADRs 0060 and 0069.
  - A Tokio runtime is built only through `goodfetch-runtime`, the one crate whose `#[expect]` lets it call `Runtime::new`, `Builder::new_current_thread` or `Builder::new_multi_thread` (issue 1221); `#[tokio::main]` is never used. Its callers are `main`, test helpers, bench targets (`benches/`, ADR 0071) and the Rust Connection, which owns one for its linked engine or its transport, or takes the test helper's paused one in its unit tests over the in-memory transport pair (ADR 0081). Engine code never builds one; `tokio::task::block_in_place` is in `disallowed-methods`.
  _Why:_ the same engine code then runs on the OS, the in-memory transport pair and the Simulation, and replays from a seed.
- **A SQLite file switches to WAL only through `goodfetch-engine`'s `sqlite_wal::enter`,** which waits out another connection's hold on the file up to the store's busy timeout; `rusqlite::Connection::pragma_update_and_check` is in `disallowed-methods` elsewhere. A test's or bench's own file that no other connection opens carries an `#[expect]` saying so.
  _Why:_ SQLite answers a journal-mode switch beside another connection `database is locked` at once, without its busy handler, so engines that made one home's file at once failed (#1773, #1774).
- **Never hold a lock across `.await`.** One exception, an agent decision of 2026-10-24 (review of spec #577, L18), which ADR 0146 ends when spec #2256 lands: a page's input order and its pointer (ADR 0085) are each a `tokio::sync::Mutex` a gesture holds from its first command to its last, because Tokio grants its lock in arrival order, the order that ADR asks for, and a gesture's commands must not interleave with another's; every wait for either, and every command sent under it, runs under its call's deadline (ADR 0080).
- **A Session, a page, the engine and a Run change state only through one transition function** ([ADR 0128](docs/adr/0128-a-session-a-page-the-engine-and-a-run-each-change-state-through-one-transition-function-that-refuses-an-invalid-transition.md)) that matches the state, then the event inside each state's arm, never a `(state, event)` tuple, with no wildcard (`clippy::wildcard_enum_match_arm` denied in the transition function's own submodule; the lint does not look inside a tuple, and review refuses an `if let`, `let … else` or `matches!` in place of the inner `match`) and returns `InvalidTransition` for a pair its table does not allow. A program-caused one gets its user-facing code (`closed`, `call_cancelled`); one only a bug can cause is an `InternalError`, never a warning that carries on. Typestates are for short-lived values such as builders.
  _Why:_ the crawlers in the research added state machines only after shipping hangs and teardown bugs.
- **Every public async function has a `# Cancel safety` section** saying what is lost if its future is dropped.
- **Public futures are `Send`**, checked by a compile-time assertion on the main entry points.

### Memory and hot paths

The absolute ceilings and Performance targets these rules serve are in [ADR 0134](docs/adr/0134-absolute-ceilings-and-performance-targets-join-the-per-change-budget-and-m1-ends-with-an-optimization-pass.md).

- **A page is parsed once, in the Page analysis; every module that reads a page is a reader of that analysis, declares what it needs, and reads its immutable result. Work that depends only on a Run's inputs is built when the Run starts, never per page. A costly per-page feature is a setting, and switching it off removes its work** ([ADR 0138](docs/adr/0138-a-page-is-analysed-once-costly-features-are-settings-and-run-state-commits-in-groups-at-a-chosen-durability-level.md)). `code-review`'s Standards axis reports a module that takes a page's HTML as a string on a page's path, and per-page work that depends only on the Run; `just counts` holds the structural counts at their ceilings ([`TESTING.md`](docs/testing/performance-budgets.md#absolute-ceilings)).
  _Why:_ the M1 profile found 8 parses, 614 selectors compiled and 135 statements prepared per page, each layer redoing what another had done.
- **Code on a Shared path opens no file, connection or configuration, compiles no selector, regular expression or validator, and prepares no statement; work that depends only on the engine's, a Run's, a Session's or a document's inputs is built once at that scope and passed in** ([ADR 0140](docs/adr/0140-every-shared-per-unit-path-has-one-owner-module-a-cost-contract-of-counted-work-and-a-benchmark.md)). The page rule above is this rule on F1. Each Shared path has one owner module and a cost contract, whose limits and Exceptions are the [Quality register](TESTING.md#quality-register)'s: read it before writing code on one. A statement is prepared through `prepare_cached`, on a connection kept for its scope: `disallowed-methods` refuses `rusqlite::Connection::execute` and `query_row`, which prepare their statement again on each call, each other use under an `#[expect]` naming its reason.
  _Why:_ work redone per unit is paid again by every page, request, entry or sample the path carries, and no single call looks costly.
- **An HTML parse goes through `CountedWork::parse_html`** (ADR 0140), which an owner takes from `Metrics::counted_work` and passes to it; a signature names the tree `ParsedHtml`. `disallowed-types` refuses `dom_query::Document`, and `disallowed-methods` `scraper::Html::parse_*`, `lol_html::rewrite_str`, `dom_query::Document::fragment` and html5ever's `parse_document` and `parse_fragment`, each other use under an `#[expect]` naming ADR 0140. `disallowed-methods` refuses `CountedWork::parse_html` too, so each parse of a page shows as an `#[expect]` naming ADR 0138 and the reader it parses for, and a second one cannot pass unseen. A selector or regular expression compiles through `CountedWork::compile_selector` or `compile_regex`, a glob through the cache's `CountedWork::glob`, and a tree reader matches a kept `Matcher` with `select_matcher`, never `select("…")`, which compiles on each call; a file opens through `CountedWork::open_file` and a folder is created through `CountedWork::create_dir_all`. A primitive beneath the engine takes the value at construction or as an argument, never from a global.
- **Bodies and frames are `Bytes`, read once and sliced, never copied into a `Vec<u8>` or a `String` on the per-request path.** Parse in place; copy only what outlives the request, such as an extracted field.
  _Why:_ a crawled page may allocate at most twice its body's bytes.
- **Every channel and every buffer between tasks is bounded,** and a full one makes its sender wait or refuse (backpressure), never grow. `mpsc::unbounded_channel` and unbounded `VecDeque` buffers between tasks are not used.
  _Why:_ an unbounded channel turns a slow consumer into unbounded memory, which no ceiling then catches until the crash.
- **No `String` clone per request or per message.** Share a value read by many owners as `Arc<str>` or an ID; borrow where the lifetime allows.
- **The Frontier's memory is bounded by `run_state.memory.budget`, and the duplicate check stays exact** ([ADR 0142](docs/adr/0142-a-runs-state-lives-in-memory-its-run-file-is-written-in-batches-and-a-clean-stop-writes-everything.md)): no Bloom, cuckoo or other lossy filter. Past the budget, or under memory pressure, what a Run-state batch already wrote leaves memory and is read back from the run file; at a budget of 0 the Frontier lives in the run file.
  _Why:_ a crawl of millions of URLs must never exhaust memory, the Frontier may hold at most 8 heap bytes per URL at a budget of 0 and past the spill (ADR 0134), and a false positive silently drops a page.
- **A collection that grows with load has a bound from a setting or the resource governor** (ADR 0040), never from the input alone.

### Lints, formatting and toolchain

- **Edition 2024, a pinned toolchain in `rust-toolchain.toml`, and the MSRV of ADR 0030 tested in CI.**
- **Warnings fail CI.** Clippy runs with `--all-targets --all-features`.
- **Clippy `pedantic` is on as warnings**, with a short allow-list kept in `[workspace.lints]`.
- **`too_many_lines` (100) and `too_many_arguments` (7) stay at their defaults** and never join that allow-list (ADR 0126); a function that needs more takes an `#[expect]` with its reason.
- **Time and order come from their handles:** `SystemTime::now`, `std::time::Instant::now`, `tokio::time::Instant::now` and library constructors that read their own clock or randomness (`Ulid::new`) are in `disallowed-methods` outside the `Clock` and `Entropy` modules, `std::env::var`, `var_os` and `vars` outside `main`, and `iter_over_hash_type` is denied (ADR 0059). The handles' test constructors `Clock::paused` and `Entropy::seeded` are in `disallowed-methods` too, opted into only by test modules (ADR 0062) and bench target roots (ADR 0071).
  _Why:_ a test can then drive every clock the code reads, and hash order never reaches output.
- **Denied:** `print_stdout` and `print_stderr`, `dbg_macro`, `todo`, `unimplemented`, `unwrap_used` (allowed in tests), `allow_attributes`, `allow_attributes_without_reason`, `clone_on_ref_ptr`, `exit` (outside `main`). `std::io::stdout` and `std::io::stderr` are in `disallowed-methods`.
- **Only three writers touch stdout or stderr:** the protocol transport, the CLI's output writer (its human output, its `--json` output and its hints on stderr) and the `tracing` subscriber's writer (stderr or a file, and stdout in `daemon run`, the foreground Daemon, where no protocol uses stdout: ADR 0049). Everything else logs through `tracing`. Test-only executables are the exceptions, each under an `#[expect]` naming its ADR: `goodfetch-scripted-engine` writes its script's messages to stdout (ADRs 0060 and 0069), `goodfetch-test-env` prints its JSON description on stdout and its failures on stderr (ADR 0062), and the counts benchmarks (`examples/counts.rs` of `goodfetch-engine`, `goodfetch-page` and `goodfetch-foxhound`, and the last's `examples/counts-extraction.rs`) print their figures on stdout and why a row goes unprinted on stderr, for `xtask counts` to read (ADR 0140).
  _Why:_ when the binary runs as an Attached engine started by an SDK, or as the stdio MCP server, stdout is the JSON-RPC channel, so one stray write corrupts the protocol; and the CLI's `--json` output is under semver (ADR 0030).
- **Exceptions use `#[expect(lint, reason = "…")]`, never `#[allow]`.**
  _Why:_ `expect` warns when the exception is no longer needed, so stale exceptions disappear.
- **rustfmt runs on its default 2024 style.**
  _Why:_ the options teams most often change (import granularity and grouping, comment wrapping) need a nightly toolchain, and defaults end style debates.
- **CI also runs `cargo-deny` (workspace-wide, and once per Product on what its executable crate ships, ADR 0087), `cargo-shear` (unused dependencies), `cargo-nextest`, `typos` and `zizmor` (workflow security).**

### `unsafe`

- **`unsafe_code` is denied across the workspace.** A module that needs it (the in-house `sandbox_init` binding of ADR 0033, the pipe file descriptors of ADR 0017) opts in with `#![expect(unsafe_code, reason = "…")]`.
- **Every `unsafe` block carries a `// SAFETY:` comment** (`undocumented_unsafe_blocks`), and `unsafe_op_in_unsafe_fn` is denied.
- **Miri runs in CI on the opted-in code it can execute (pure-Rust `unsafe`).** FFI and system-call modules, which Miri can't run, are covered by tests on each real OS, and by the Release suite's AddressSanitizer run on Linux (ADR 0067).

### Logging and secrets

- **`tracing` everywhere, never `println!` or `log`.** Spans cover each RPC request, run, page and CDP command, with glossary IDs as fields (`run_id`, `session_id`).
- **The RPC layer opens every request's span:** its dispatcher one per request received, from the catalog entry, and one per request sent, whose Trace context it writes into `_meta` ([ADR 0132](docs/adr/0132-each-request-may-carry-its-callers-trace-context-across-goodfetchs-own-links-and-the-engines-spans-stay-experimental.md)). A handler opens only its own child spans, never a request span.
- **Span fields hold bounded values only:** IDs, operation names, Error codes and values from bounded sets. A URL is recorded as its origin; params, page content, extracted data and model prompts and completions never reach a span.
  _Why:_ an OTLP export is one more output that leaves the process, and a query string can hold a token.
- **`#[instrument(skip_all, fields(…))]`**, so arguments are only logged when named.
  _Why:_ one `?config` in a log line can leak a proxy password, and secrets never reach logs (ADRs 0014, 0015).
- **Logs go to stderr or a file, never stdout**, through the `tracing` subscriber; the one exception is `daemon run`, the foreground Daemon, which logs JSON lines to stdout because no protocol uses its stdout and container platforms collect stdout (ADR 0049).
- **Levels:** `error` needs someone to act; `warn` is a degraded but handled state; `info` is a lifecycle step of a run, Session or Daemon; `debug` explains a decision; `trace` is per-message detail.
- **Credentials, cookies, tokens and keys are `secrecy::SecretString`** (or a wrapper with a hand-written `Debug`). Secret canaries and redaction property tests prove the secret reaches no output ([`TESTING.md`](docs/testing/security.md#secrets), ADR 0067).

### Dependencies

A crate or package becomes a dependency only when it meets this bar:

- its license is on the allowlist of ADR 0018;
- it builds on every target: static musl (ADR 0024), macOS and Windows;
- it is maintained, or deliberately stable;
- it has no open security advisory;
- it supports our MSRV;
- its build script downloads nothing.

For anything non-trivial, a known and widely used project is preferred to writing it in-house. Versions live in `[workspace.dependencies]` with `default-features = false`, and each crate turns on what it uses. Every member also takes `goodfetch-workspace-hack`, which `ci/workspace-hack.sh generate` writes after a dependency or feature change and `just lint` checks (ADR 0094). `cargo-deny` and the npm and Python equivalents (ADR 0032) enforce what they can.

### Cargo features

- **A capability is never a Cargo feature** (ADR 0038): stealth, the Challenge solver, AI and costly controls are settings. The Products differ only by the crates their executables link, never by a feature (ADR 0087).
- **Features exist only on the published crates through which each Product's SDK embeds its engine** (such as `goodfetch-collie` and `goodfetch-foxhound`, ADR 0090), only for heavy optional dependencies and `test-util`, and they are additive. Each Product's executable builds one fixed set in its own `cargo build -p`, and `cargo-hack` checks every combination of each embeddable crate's features.
- **Platform differences use `cfg(target_os = …)`, never features.** Custom `cfg`s are declared for `unexpected_cfgs`: `goodfetch_loom` for loom tests (ADR 0060), and `tokio_unstable` only because the Simulation's scenarios compile under it, in a job of their own. Under `goodfetch_loom`, a crate's `sync` module maps its atomics and locks to loom, and `disallowed-types` refuses the std atomic, `Mutex`, `RwLock` and `Condvar` types outside it.

### Tests

The test rules live in [`TESTING.md`](TESTING.md) (where tests live, how they are written and named, shared helpers, fixtures, snapshots, injection and determinism, simulation, model checking and crash testing, flaky tests and Live checks, the test-first process, suites, AI features and security tests; ADRs 0059, 0060, 0061, 0062, 0064, 0065 and 0067), apart from Design's "Tests go through the interface" and each language's runner choices below.

### Documentation in code

- **`missing_docs` warns on the public crates and `goodfetch-protocol`.** The public crates are `goodfetch-collie`, `goodfetch-foxhound` and `goodfetch-collie-client` (ADRs 0090 and 0095).
- **Each public function of the public crates and `goodfetch-protocol` documents `# Errors`** (with the Error codes it can return), **`# Cancel safety`** if it is async and **`# Panics`** if it can panic; on the public crates it also has **an example** that runs as a doctest.
- **Internal crates document what a caller outside the module must know**: their `//!` headers, `# Errors` where a caller would otherwise guess, and `# Cancel safety` on every public async function (ADR 0038).
- **The catalog's description of an operation is its only text source:** MCP tool descriptions, SDK docstrings and reference pages are generated from it.
  _Why:_ what a model reads in a tool description directly affects agent accuracy, and the surfaces then never disagree.
- **Comments say why, never what the code already says.**

## Protocol and SDKs

- **The SDK API follows the principles of [ADR 0077](docs/adr/0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md)**: eight guarantees (G1–G8), four ranked shape rules (S1–S4) and a tie-breaker (T), cited by ID.
- **Contracts are Rust types in `goodfetch-protocol`**, emitted as a committed JSON Schema bundle (ADR 0038). To add or change an operation, edit the types, run `xtask codegen`, and commit what it produces.
- **Every catalog entry names the smallest Product that carries it**, Collie or Foxhound ([ADR 0095](docs/adr/0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md)). Each TypeScript and Python SDK package generates only its own Product's entries: each language's Collie client package (`collie-client/`) Collie's, Foxhound's package Foxhound's, on top of the client package. Each Rust SDK crate re-exports only its own Product's entries of `goodfetch-protocol`.
- **Generated code is never edited by hand.** It lives in a `generated/` folder in each SDK, and CI fails when it differs from what `xtask codegen` produces.
- **Hand-written code in each SDK is the runtime and the ergonomic layer:** engine start and discovery, framing, `hello`, sockets, request correlation, flow-control credits, event resumption, file-backed bodies, cancellation, and the objects users hold. It lives in the Collie client package, except each Product's connection type, which finds its own executable, and the objects only Foxhound hands out, which live in Foxhound's package.
- **Every public hand-written member is listed in its SDK's manifest** with the catalog entry it serves, or none and a reason; it carries that entry's doc text and markers, and a member the generator could express is generated instead ([ADR 0099](docs/adr/0099-every-surface-spells-each-catalog-name-by-its-own-rule-shapes-come-from-declared-kinds-and-call-is-the-way-down.md)). `xtask sdk-manifests` fails on a member whose entry is no catalog operation or one only a person may call, such as `usage.raise` (ADRs 0041, 0080), on two members of one receiver for one entry, on a member with no entry not spelled from its words and form, on a line naming a member the generator writes, and on a member another SDK's manifest lacks on the same receiver unless its `only` says why. Matching a manifest to the members an SDK exports, with their doc text and markers, is issue 2920's, so until then the listing itself is kept by hand and by review.
  _Why:_ hand-written ports drift from what they port; once the export match runs, a listed member is checked, and an unlisted one fails the build.
- **Among the npm and PyPI packages under `sdks/`, `adapters/` and `integrations/`, Foxhound's depend on no Collie package but the Collie client package, Collie's on no Foxhound package, and the client package on neither Product's package** (ADR 0095). How Foxhound's crates use Collie's is ADR 0087's. Programs import from their Product's package; only code meant to serve both Products imports the client package.
- **Settings reach code through one `settings` option per scope, generated from the settings schema** ([ADR 0083](docs/adr/0083-a-program-sets-settings-in-one-typed-option-explicit-values-beat-the-environment-and-no-value-is-silently-ignored.md)): a nested object literal in TypeScript, a nested `TypedDict` in Python, typed key constants on `Settings` in Rust. No SDK reads the environment outside `connect()`'s environment map; in Rust, that read of the process environment carries the one `#[expect]` of `disallowed-methods` outside `main` (ADR 0059).
- **The wire is snake_case.** Python uses wire names as they are, a keyword taking a trailing underscore (`continue_`, ADR 0099); TypeScript presents camelCase through the generated mapping, which never renames user data (extracted records, headers, metadata maps).

## TypeScript

Applies to `sdks/typescript`, `adapters/`, `integrations/`, `bench/` and the docs site's `website/` in the pnpm workspace.

- **TypeScript 7 with `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax`, `isolatedDeclarations` and `erasableSyntaxOnly`.** A TypeScript 6 alias is kept only for tools that need the compiler API (API Extractor, TypeDoc, Twoslash) until TypeScript 7 provides one.
- **Optional fields are written `?: T | undefined`.**
  _Why:_ the published types then work whether or not the consumer enables `exactOptionalPropertyTypes`.
- **oxlint with type-aware rules, and oxfmt for formatting.** `no-floating-promises` and `typescript/no-deprecated` are errors.
  _Why:_ an unawaited promise loses its error silently; oxlint's type-aware rules run on the real TypeScript 7 checker.
- **Libraries build with tsdown. Outside `website/`, tests run with Vitest on every supported Node line (ADR 0024), plus smoke scripts of the installed package on Bun, Deno 2 and the edge runtimes (ADR 0069).**
- **The docs site is tested in browsers, on one Node line:** Vitest Browser Mode and Playwright Test (ADR 0068, rules in [`TESTING.md`](TESTING.md)).
- **Packages are ESM-only** (ADR 0024) and pass `publint` and `@arethetypeswrong/cli` before publishing. npm is the only registry.
- **Named exports only; no TypeScript `enum`s** (string-literal unions instead).
- **SDK idioms:**
  - every operation that can be cancelled takes an `AbortSignal` option;
  - engines and Sessions are disposed with `await using`;
  - sequences a program reads to their end are `AsyncIterable`, and side activity on a page is observed with `on`, which returns a `Subscription` (ADR 0081);
  - engine responses are not validated at runtime beyond the protocol version check;
  - schemas users pass in (for extraction, Datasets and Payloads) are accepted as Standard Schema with Standard JSON Schema (the interfaces Zod and ArkType implement, and Valibot through its adapter), whose types the SDK copies rather than depending on a schema library, and converted to JSON Schema (ADR 0082).
- **Errors are one class per Error code, generated.**
- **Doc comments are TSDoc.**

## Python

Applies to `sdks/python`, the Python adapters and the uv workspace. Supported versions follow ADR 0024: every CPython not past end of life, including free-threaded builds.

- **pyright in strict mode gates CI**, and mypy `--strict`, with the `deprecated` error code on, checks the generated code and the type tests that exercise the public API (`TESTING.md`, ADRs 0069 and 0099).
  _Why:_ pyright is what most editors show; the mypy check keeps the types right for mypy users too.
- **ruff formats and lints**, with the rule sets `E F I UP B C4 ARG T20 TID251 PERF RUF ANN PYI TC ASYNC D`, ignoring `D206` and `D300`, which conflict with ruff's formatter.
  _Why:_ `ASYNC` catches blocking calls inside async code, and `T20` bans `print`.
- **Pure-Python packages build with `uv_build`; every package ships `py.typed`.** The SDK's `py3-none` platform wheels, which carry the engine binary (ADR 0024), are built by the release job of ADR 0032.
- **The async core uses anyio**, so the async API runs on asyncio and trio. The sync API is generated and runs that core on its own event-loop thread (ADR 0038).
- **Protocol data are pydantic v2 models, generated; errors are one exception class per Error code, generated.**
- **Tests use pytest with the anyio plugin** (every async test runs on asyncio and trio), `inline-snapshot` for protocol payloads and `hypothesis` for encode and decode round-trips. CI also runs the free-threaded build.
- **Docstrings follow the Google style**, enforced by ruff's `D` rules, for Griffe (ADR 0030).
