# SDK API sketch

The API sketch of the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461): typed signatures for the TypeScript, Python and Rust SDKs, with no implementation, plus the golden scenarios written against them. It lives on the `prototype/sdk-api` branch and is never merged. Each grilling ticket on the map adds the signatures and scenario snippets it decides, so elegance is judged on code while deciding, and links its commit from its resolution.

## Layout

```text
sdk-api-sketch/
├── check.sh            # type-checks all three languages
├── typescript/
│   ├── src/            # .d.ts declarations, re-exported from index.d.ts
│   └── scenarios/      # golden scenarios as .ts programs importing "goodfetch"
├── python/
│   ├── goodfetch/      # .pyi stubs, re-exported from __init__.pyi
│   └── scenarios/      # golden scenarios as .py programs
├── rust/
│   ├── src/            # modules re-exported from lib.rs
│   └── examples/       # golden scenarios as examples
├── scenarios/          # one description per golden scenario: task, leader, pains, failure points, measures
│   └── leaders/        # the leaders' programs, with their own package manifests
└── pain-coverage.md    # every researched pain and how the SDK API answers it
```

[`scenarios/README.md`](scenarios/README.md) lists the twelve golden scenarios and how they are measured. [`pain-coverage.md`](pain-coverage.md) is the pain coverage matrix.

## Conventions

- **One file per ticket topic** in each language, for example `typescript/src/lifecycle.d.ts`, `python/goodfetch/_lifecycle.pyi` and `rust/src/lifecycle.rs`, re-exported from the language's entry file.
- **Each ticket fills its matrix rows.** A grilling ticket fills its rows of `pain-coverage.md` (closes as, feature, scenarios, principles) in the same commit as its signatures and snippets.
- **Snippets.** A ticket's usage examples sit beside the golden scenarios, type-checked with them, until #483 writes the programs: `typescript/scenarios/snippets/<topic>.ts`, `python/scenarios/snippet_<topic>.py` and `rust/examples/snippet_<topic>.rs`.
- **Signatures only.** Rust bodies are `unimplemented!()`, the sketch's one departure from `STYLE.md`.
- **Placeholder names.** The package and import name `goodfetch` stands in until the packaging question of [Goodfetch products: Collie and Foxhound as separately installable products](https://github.com/goodfetch/automan/issues/447) settles names.
- **Checkers follow `STYLE.md`:** TypeScript 7 with `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax` and `erasableSyntaxOnly` (`isolatedDeclarations` only applies when emitting declarations, which a sketch of `.d.ts` files never does); pyright strict and mypy `--strict` on Python 3.11; `cargo check` with warnings denied.

## Checking

```sh
./check.sh
```

It needs Node 22 or later, uv and a Rust toolchain, and fetches the pinned `tsc`, pyright and mypy versions on first run.

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
