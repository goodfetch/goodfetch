# Each crate declares which of its modules may import which, each module names one concern, and a file past 1,000 lines states why

Inside a crate, separation of concerns is checked, not assumed. Each crate lists which of its modules may import which sibling modules, and an `xtask` check fails the Quick suite on any other import, on a cycle and on a rule that names a module which no longer exists. Each module's `//!` comment names one concern. A file past 1,000 lines fails the same check unless its header states why it is still one concern: an alarm, not a cap. Clippy's function-size lints stay at their defaults.

Crates enforce direction between crates, each for a reason `STYLE.md` states (ADR 0087). Inside a crate, Rust's visibility decides who outside the crate sees an item, but not which sibling module may use which, and nothing else did. Some concerns were already pinned to one module by `disallowed-methods` and `disallowed-types` (sockets to `transport`, spawning to `tasks`, clocks and randomness to `Clock` and `Entropy`, environment reads to `main`), and Clippy `pedantic` with warnings failing CI already caps a function at 100 lines. The [competitor codebase design research](https://github.com/goodfetch/goodfetch/blob/research/competitor-codebase-design/rnd/research/reports/Competitor%20codebase%20design.md) ([#555](https://github.com/goodfetch/goodfetch/pull/555)) read seven projects at their heads of 15 October 2026 (Spider at 1 October):
- **Every project has one central class that every feature passes through, and it grew by mixing concerns.** Crawlee's crawler owns retries, sessions, robots.txt, rate limits, statistics, root-logger setup, signal handling and CSV export, each added where it was easiest. Spider's `impl Website` is 13,435 lines with 361 functions; Browser Use's `beta/service.py` 6,810 lines; Crawlee JS's `BasicCrawler` 3,180; Stagehand's god object survived its v4 rewrite one layer down, in a 2,770-line `Page`. Median files are 114 to 184 lines in Crawlee Python, Playwright, Scrapy and Browser Use (Spider's is 335); every one of those central classes or files is over 1,000.
- **Import rules hold where a tool checks them.** Playwright's 51 `DEPS.list` files, checked by `check_deps.js` in its lint, keep its client/server split and its composition root. Its two holes: type-only imports are exempt, and five rules name files that no longer exist without the checker noticing. Without such a check, Scrapy's `utils` sits in an import cycle across 17 packages and modules, Crawlee JS has four cycles inside packages, and Browser Use's `agent` and `tools` import each other.
- **Size rules written but unchecked do not hold, and hard caps are rare.** Browser Use's contributor guide says to split files past about 500 lines, and 14 of its files are over 1,000. Crawlee Python selects every ruff rule and disables every size rule. ESLint's `max-lines` is off by default. rust-lang/rust's `tidy` flags a file past 3,000 lines unless the file carries an `ignore-tidy-filelength` comment.

Decided in [Separation of concerns inside a crate is unchecked](https://github.com/goodfetch/goodfetch/issues/570) ([grilling record](https://github.com/goodfetch/goodfetch/issues/570#issuecomment-6214829950)). The owner decided the rules; the agent decided their details on the owner's delegation of 2026-10-16. No SDK API changes, so ADR 0077's principles do not apply.

## Decision

### Module import rules

- **Each crate has a `modules.toml` beside its `Cargo.toml`.** It lists every top-level module of the crate (each module `lib.rs` or `main.rs` declares) with the sibling modules it may import, an empty list for one that imports none:

  ```toml
  [modules]
  retry = ["clock", "error"]
  dispatch = ["retry", "error"]
  error = []
  ```

  A module missing from the file fails the check. A module's own submodules are its business; a module that grows submodules worth rules of their own adds a `[submodules.<name>]` table in the same file, listing its submodules the same way. A submodule's imports of its parent's siblings count against its parent's line:

  ```toml
  [submodules.dispatch]
  route = ["table"]
  table = []
  ```
- **What counts as an import:** any path that reaches a sibling, whether through `use`, a qualified path in code, or a type in a signature: `crate::x`, `super::x` resolving to a sibling, and re-exports. **Type-only imports are not exempt**, nor paths inside macro invocations such as `tracing::info!`, which the check parses as tokens since `syn` leaves them unparsed; [#132](https://github.com/goodfetch/goodfetch/issues/132) builds it so. Generated code (`generated/` folders, ADR 0038) is not checked, and generated bindings sit under one top-level module. `lib.rs` only declares modules and re-exports (`STYLE.md`), and `main.rs` is the composition root (ADR 0087); neither is checked. Code under `#[cfg(test)]` is not checked: a test may build its fixtures from any module of its crate, and it still calls the module under test through its interface (ADR 0062).
- **The `xtask` check fails** on an import the file does not allow, on a cycle among the allowed imports, on a name in the file that is not a module of the crate, and on a module the file does not list. It runs in the Quick suite (ADR 0061) and parses source with `syn`, so it needs no build.
- **It adds to the existing pins.** A module that `disallowed-methods` or `disallowed-types` already reserves a concern to (`transport`, `tasks`, `Clock`, `Entropy`, `sync`) keeps that reservation; the file only says who may call it.

### One concern per module

- **A module's `//!` comment names one concern**, a review rule. A role that needs "and" to describe two jobs is two modules. A module that implements one concern through several parts (a decoder and its error table, a state machine and its transitions) is still one.

### A size alarm, not a cap

- **A Rust source file under a crate's `src/` over 1,000 lines fails the `xtask` check unless its `//!` header holds a line `Size: <why this is still one concern>`.** Test code is not counted: files under `tests/` and `benches/` are not checked, and the count leaves out items under `#[cfg(test)]`, a file such as `foo/tests.rs` that only a `#[cfg(test)] mod tests;` declares included. Generated code (`generated/` folders, ADR 0038) is not counted either. Every other line counts, blank lines and comments included, as the research's sizes do. The line is read by the review like any reason under `#[expect]`.
- **There is no alarm on a type's number of methods.** The import rules and the one-concern rule reach the cause; a method count would flag a deep type as readily as one that mixes concerns.
- **The rule covers the Rust workspace**, the Rust SDK crates included. The hand-written code of the TypeScript and Python SDKs is their runtime and ergonomic layer, listed member by member in each manifest (ADR 0099); it is not checked by size.

### Function size

- **`too_many_lines` (100) and `too_many_arguments` (7) stay at Clippy's defaults** and never join the `[workspace.lints]` allow-list. A function that needs more opts out with `#[expect(clippy::too_many_lines, reason = "…")]` (`STYLE.md`).

## Considered Options

- **Review alone, with no import rules:** rejected. Scrapy, Crawlee JS and Browser Use, which have no such check, each have import cycles, and Browser Use's written size rule did not hold either.
- **Rules in each module's `//!` comment:** rejected. Parsing prose is fragile, and the rules for a whole crate are easier to review in one file.
- **A rules file per module, as Playwright's `DEPS.list` per folder:** rejected. Rust's modules are files inside one crate, and one file per crate keeps the cycle check whole.
- **A hard cap on file length:** rejected. It forces a split at an arbitrary line, which spreads one concern over two files and leaves mixed concerns mixed. A deep module (`STYLE.md`) is sometimes long.
- **No size rule:** rejected. It loses the early warning every central class in the research would have raised.
- **A lower threshold, such as Browser Use's 500 lines:** rejected. It would fire on files that hold one concern; 1,000 catches every central class in the research.
- **Cargo crates instead of modules for every boundary:** rejected. `STYLE.md` makes a crate only for four reasons, since each crate adds visibility plumbing.

## Consequences

- **`STYLE.md`.** Workspace and crates states the import rules and the size alarm; Design states the one-concern rule; Lints states that the two function-size lints stay on.
- **Review.** The one-concern rule and each `Size:` reason are review rules in `STYLE.md`, checked by the fresh review of a pull request; the `xtask` check holds the rest.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): no fresh review exists; this check is a review standard the `code-review` skill applies, and the owner sees its result before merging.
- **deslop-please joins the check** (note of 2026-11-26, the owner's choice that day). `just deslop-please` runs deslop-please 0.1.6 through `ci/deslop-please.sh`, once per spec, by its implementer in its own slot at the spec's end before `code-review`, and as a step of the full gate, which runs only on the owner's `just merge <n> --full`; the quick gate never runs it, as it indexes the whole workspace again after any change (about 6 minutes and 10 GB, measured that day; 2 seconds on a tree it has indexed). It fails on a module cycle, a tangle of a module and its child, a struct field used outside its owner or a shallow module that `ci/deslop-please.allow` does not list. The list holds what `main` and the open spec branches held that day, and it only shrinks, as `ci/copies.json` does (TESTING.md, Quality register). deslop-please's `long-file` (over 2,000 lines) and `wide-directory` (over 15 entries) are not checked: they are the hard caps this ADR rejected, and the `Size:` alarm stays the file-length rule. Spec [#4005](https://github.com/goodfetch/goodfetch/issues/4005) fixes the listed findings crate by crate.
- **ADR 0061** carries a note: the Quick suite runs this check.
- **ADR 0037.** Area 1 (Repository, CI and release) lists this ADR: [Workspace skeleton and the "all green" pull-request check](https://github.com/goodfetch/goodfetch/issues/132) builds the `xtask` check. Every area's crates follow it, as they follow ADR 0038.
