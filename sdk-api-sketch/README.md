# SDK API sketch

The API sketch of the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461): typed signatures for the TypeScript, Python and Rust SDKs, with no implementation, plus the golden scenarios written against them. It lives on the `prototype/sdk-api` branch and is never merged. Each grilling ticket on the map adds the signatures and scenario snippets it decides, so elegance is judged on code while deciding, and links its commit from its resolution.

## Layout

Each language has ADR 0095's three packages: the Collie client package, with every value a Collie connection hands out and no executable; Collie's package, with the `Collie` connection; and Foxhound's, with the `Foxhound` connection and what only Foxhound hands out (crawls, Datasets, Records, Exports, Hooks, Politeness toward sites, the crawl settings and Error codes). Both Products' packages re-export the client package, so a handler's Page and a Collie program's Page are one type.

```text
sdk-api-sketch/
├── check.sh                 # type-checks all three languages, then the leaders' programs
├── manifest.md              # the hand-written members of each SDK (ADR 0099)
├── typescript/
│   ├── collie-client/       # @goodfetch/collie-client (+ /testing): .d.ts re-exported from index.d.ts
│   ├── collie/              # @goodfetch/collie (+ /testing): Collie
│   ├── foxhound/            # @goodfetch/foxhound (+ /testing): Foxhound and its own values
│   └── scenarios/           # golden scenarios as .ts programs; snippets/ holds each ticket's snippets
├── python/
│   ├── goodfetch_collie_client/  # .pyi stubs, re-exported from __init__.pyi; testing.pyi
│   ├── goodfetch_collie/         # Collie, AsyncCollie; testing.pyi
│   ├── goodfetch_foxhound/       # Foxhound, AsyncFoxhound and Foxhound's own values; testing.pyi
│   └── scenarios/                # golden scenarios as .py programs, beside the snippet_*.py snippets
├── rust/                    # a workspace; its own package holds the scenarios as examples
│   ├── collie-client/       # goodfetch-collie-client
│   ├── collie/              # goodfetch-collie: Collie, blocking::Collie, testing
│   ├── foxhound/            # goodfetch-foxhound: Foxhound, error_code, settings, testing
│   └── examples/            # golden scenarios (sNN_slug.rs) and snippets (snippet_*.rs)
├── scenarios/               # one description per golden scenario: task, leader, pains, failure points, measures
│   └── leaders/             # the leaders' programs, with their own package manifests
└── pain-coverage.md         # every researched pain and how the SDK API answers it
```

[`scenarios/README.md`](scenarios/README.md) lists the twelve golden scenarios and how they are measured. [`pain-coverage.md`](pain-coverage.md) is the pain coverage matrix.

## Conventions

- **One file per ticket topic** in each language, for example `typescript/collie-client/lifecycle.d.ts`, `python/goodfetch_collie_client/_lifecycle.pyi` and `rust/collie-client/src/lifecycle.rs`, re-exported from the package's entry file; a topic with Foxhound's own part has a file of the same name in Foxhound's package (`foxhound/settings.d.ts`, `goodfetch_foxhound/_settings.pyi`, `foxhound/src/settings.rs`).
- **Each ticket fills its matrix rows.** A grilling ticket fills its rows of `pain-coverage.md` (closes as, feature, scenarios, principles) in the same commit as its signatures and snippets.
- **Golden scenarios and snippets.** The golden scenarios are `typescript/scenarios/NN-slug.ts`, `python/scenarios/sNN_slug.py` and `rust/examples/sNN_slug.rs` (scenarios/README.md). A ticket's usage examples stay beside them as snippets, type-checked with them, for the shapes no golden scenario shows: `typescript/scenarios/snippets/<topic>.ts`, `python/scenarios/snippet_<topic>.py` and `rust/examples/snippet_<topic>.rs`.
- **Signatures only.** Rust bodies are `unimplemented!()`, the sketch's one departure from `STYLE.md`.
- **The testing kits are their own entries** (ADRs 0069, 0090): `@goodfetch/collie/testing` and `@goodfetch/foxhound/testing` (with the shared `@goodfetch/collie-client/testing`), `goodfetch_collie.testing` and `goodfetch_foxhound.testing`, and `goodfetch_collie::testing` and `goodfetch_foxhound::testing` behind the `test-util` feature, which `check.sh` turns on with every other feature. Foxhound's adds `runHandler` and `runHook`.
- **Package names follow ADRs 0090 and 0095.** A program imports only its Product's package; only a library meant to serve both imports the client package. The connection is the Product (`Collie.connect()`, `AsyncCollie.connect()`, `Collie::connect(…)`, Foxhound's likewise), and both implement `Connection`, which carries Collie's Run start. In the SDK, Rust's catalog types come from `goodfetch-protocol`, Foxhound's from its `foxhound` module (ADR 0099); the sketch defines them in the crates themselves. Where a name both packages define means more in Foxhound's (`Settings`, `SettingKey`, `TimeoutSettings`, `KnownErrorCode`, `ConnectOptions`), Foxhound's package gives its own, which adds its keys or values to Collie's.
- **Names follow ADR 0099** ([Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471)).
  - **One rule per language.** Every name is the catalog's snake_case words, spelled by one rule per language:
    - TypeScript: camelCase members and PascalCase types;
    - Python: snake_case members and PascalCase classes, with an `Async…` twin of every class with a round trip;
    - Rust: snake_case, UpperCamelCase types and UPPER_SNAKE setting constants.
  - **Initialisms.** One from the closed list (`ai api cdp css dom html http https json mcp pdf uri url xml`) is in capitals in TypeScript names and Python class names (`waitForURL`, `CDPSession`), lowercase when it opens a camelCase name (`url()`), and one word in Rust (`CdpSession`). `id` is a word (`runId`).
  - **Keywords.** TypeScript escapes none (`route.continue()`), Python adds a trailing underscore (`continue_`), and Rust uses a raw identifier (`r#continue`).
  - **Reads the SDK answers itself on a handle.** In TypeScript they are a method, as all of Playwright's are (`page.url()`, `connection.engine()`); a sub-object reached without I/O is a `readonly` property (`page.keyboard`). In Python a read with no arguments is a property, except `is_`/`as_` names. In Rust they are a method.
  - **The raw catalog call** is `call(operation, params)` on the connection and on every handle (`catalog` files), with `params` a builder method in Rust (`call(op).params(…)`). The sketch shows it on the Run, Session, Page and Locator.
  - **Open value types in Rust.** `ErrorCode`, `LimitScope`, `UsageKind` and `HookStage` are newtypes holding their wire string inline, Collie's values as associated constants and Foxhound's as free constants (`goodfetch_foxhound::error_code`, `limit_scope`, `usage_kind`, `hook_stage`); an Error code's details are read with `error.details::<D>()`.
  - **Hand-written members** are listed in [`manifest.md`](manifest.md) with the entry each serves, or their words and form; ADR 0089's lending helpers are free functions of the client package (`lendToPlaywright`, `lendToPuppeteer`, `lend_to_playwright`, `lend_to_browser_use`).
- **Checkers follow `STYLE.md`:** TypeScript 7 with `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax` and `erasableSyntaxOnly` (`isolatedDeclarations` only applies when emitting declarations, which a sketch of `.d.ts` files never does); pyright strict and mypy `--strict` on Python 3.11; `cargo check` with warnings denied.

## Checking

```sh
./check.sh
```

It needs Node 22 or later, uv and a Rust toolchain, and fetches the pinned `tsc`, pyright and mypy versions on first run, with the libraries the scenarios use: Zod, `@types/node` and Playwright (scenario 09's own program) in TypeScript; Pydantic, FastAPI, pytest and Playwright in Python; `schemars`, serde, Tokio and axum in Rust. It then type-checks the leaders' programs against their real packages (`scenarios/leaders/`), downloading no browser.

## Review list

Every ticket's shapes are reviewed against the principles of [ADR 0077](https://github.com/goodfetch/automan/blob/main/docs/adr/0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md), cited by ID. The ADR holds the full text and evidence.

**Guarantees**, never traded for a shorter or more familiar shape:

- **G1 Honest results:** a failure never looks like a result, an expected outcome is never thrown, and "nothing happened" says why.
- **G2 No hidden state:** every call acts on a value the caller holds, with no process-wide default instance.
- **G3 Strict inputs, tolerant results:** unknown or ineffective options fail, and unknown result fields and enum values are kept.
- **G4 A guest in the user's program:** it never owns the process, the host's event loop, signal handlers or stdout; logs go through the host's logging; it never closes what it did not start.
- **G5 Safe defaults for the user and for the sites reached:** anything less safe is an explicit, recorded opt-in.
- **G6 Diagnosable:** every failure says what was attempted, what was observed, where the limit came from and what would fix it; every call made in a run is visible in its Run trace.
- **G7 Goodfetch's own types are enough:** no third-party type is the only way to use an operation.
- **G8 A way down at each layer:** each layer has a documented way down, and no golden scenario needs one.

**Shape rules**, the higher one winning a conflict:

1. **S1 Idiomatic and consistent within each language**, identical in meaning across them.
2. **S2 One obvious way per task:** settings carry the rest, shortcuts are built on the same objects, and a method has one return type.
3. **S3 Familiar where the meaning is the same:** a different meaning gets a different name or type.
4. **S4 The same form across languages**, ranked last.

**Tie-breaker:**

- **T Written for coding agents too:** ties go to the shape that scores better in the agent-writability check and the golden scenarios, and every deprecated name carries a type-level marker.
