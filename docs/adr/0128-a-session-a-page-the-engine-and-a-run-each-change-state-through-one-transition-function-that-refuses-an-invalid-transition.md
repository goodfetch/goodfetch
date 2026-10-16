# A Session, a page, the engine and a Run each change state through one transition function that refuses an invalid transition

Each long-lived object an actor owns keeps its lifecycle as a state enum and changes it only through one transition function, which matches every pair of state and event with no wildcard and returns an error for a transition that is not allowed. When a program caused the invalid transition, the program gets the user-facing code that already exists for it; when only a bug in Goodfetch can cause it, it is an `InternalError` logged at `error`. It is never a warning that carries on. Every pair of state and event is a named test case. Short-lived values, such as builders, may use typestates instead.

Ownership was decided: every task starts under its owner's tracker, state with one owner is an actor, and an SDK program closes only what it started (`STYLE.md`, ADRs 0038 and 0078). A Run's Lifecycle states were decided (`running`, `stopped`, `finished`, `failed`, ADR 0029), with how a Run resumes or is refused while another writer holds it (ADR 0102), and `run_state` transitions are tested against reference models (ADR 0060). Some page events were decided too: a page closed under a call (ADR 0113) and a navigation an AI verb did not start (ADR 0114). Not decided: how code refuses an invalid transition, and the states of a Session, a page and the engine. The [competitor codebase design research](https://github.com/goodfetch/goodfetch/blob/research/competitor-codebase-design/rnd/research/reports/Competitor%20codebase%20design.md) ([#555](https://github.com/goodfetch/goodfetch/pull/555)) found the crawlers fixing this late, after shipping bugs:
- **Scrapy** added an engine state machine on its unreleased main branch after years of hangs on early shutdown (scrapy/scrapy#6916), and an invalid transition still only logs a warning.
- **Crawlee JS** retrofitted owned-versus-injected collaborators and a per-run `CrawlerRun` object for its v4 major, after a teardown bug made multi-run tests "process nothing" (apify/crawlee#3087).
- **Crawlee Python's** crawler enters any collaborator not yet `active`, so a shared instance breaks; the fix is planned, not shipped (apify/crawlee-python#2301).

Decided in [An invalid lifecycle transition has no refusal rule](https://github.com/goodfetch/goodfetch/issues/572) ([grilling record](https://github.com/goodfetch/goodfetch/issues/572#issuecomment-6214830642)). The owner decided the rules; the agent decided their details on the owner's delegation of 2026-10-16. No SDK API changes, so ADR 0077's principles do not apply: the codes a program sees are ones that already exist.

## Decision

### One transition function

- **The objects:** a Session, a page, the engine (an Attached engine and a Daemon) and a Run. Each area spec lists its object's states, its events and the table of allowed transitions under this rule: area 3 (Engine runtime) for the engine and the Run, area 5 (Site access) for a Session and a page. An area spec may put another object an actor owns under the same rule.
- **The state is an enum the actor owns.** It changes only through one function, such as `fn apply(&mut self, event: Event) -> Result<Effects, InvalidTransition>`, in the module that owns the object (ADR 0126).
- **The function matches the state, then the event inside each state's arm, with no wildcard arm in either.** It sits in a submodule of its own that turns on `#![deny(clippy::wildcard_enum_match_arm)]`, so the lint refuses a wildcard in either `match` without banning `_` on the module's other enums. The inner dispatch is a `match`: an `if let … else`, a `let … else` or a `matches!` would let a new event through unseen, and the lint does not see them, so review refuses them. A single `match (state, event)` on a tuple is not allowed: the lint does not look inside a tuple pattern, so `(_, event)` and `_` arms would pass it (checked with Clippy 0.1.93).
- **`InvalidTransition` names the state and the event.**
- **Typestates are allowed for short-lived values a caller holds and consumes**, such as a builder, where each state is its own type and an invalid call does not compile. An object whose state changes as messages reach its actor keeps the enum.

### What an invalid transition does

- **Caused by a program**, such as a call on a page after it was closed: the program gets the user-facing code that already names its remedy, which is `closed` for a value the program closed (ADR 0078), `call_cancelled` for a call whose page or browser closed under it (ADR 0113), or the code its area spec names.
- **Only a bug can cause it**, such as an event from the engine's own code in a state that cannot receive it: a code in ADR 0079's `InternalError` group, logged at `error` with the state and the event.
- **Never a warning that carries on.** Under the Simulation and in tests an `InternalError` fails the test, as any other.

### Tests

- **Every pair of state and event is a named test case** (rstest's `#[case::name]`, `TESTING.md`), with its expected outcome copied from the area spec's table: the next state and effects, or the code. No loop computes an expected outcome.
- **A Run keeps its reference models** (ADR 0060), and the engine's states join the Simulation's scenarios where a crash or a lost connection drives them.

## Considered Options

- **Typestates for every object:** rejected. A state that changes as messages reach an actor is held in one place across awaits, which a type per state cannot express without moving the value out on each message.
- **A flag per condition** (`active`, `closing`), as Crawlee Python: rejected. Flags allow combinations no table lists, which is how a shared collaborator broke there.
- **Log a warning and carry on**, as Scrapy: rejected. The object is then in a state no one designed.
- **Panic on an invalid transition:** rejected. A program can cause some of them, and `STYLE.md` keeps panics for bugs that input cannot trigger.
- **A wildcard arm returning `InvalidTransition`:** rejected. A new state or event would then be refused silently instead of decided.
- **One `match (state, event)` with a review rule against wildcards:** rejected. The lint cannot hold it, and a wildcard in a tuple pattern is easy to miss in review.

## Consequences

- **`STYLE.md`.** Async and concurrency states the rule; Design's "state with one owner is an actor" names who owns the state.
- **`TESTING.md`.** The review checklist checks that a new state or event adds its named cases.
- **ADR 0037.** Areas 3 (Engine runtime) and 5 (Site access) list this ADR. Their specs list the states, events and tables.
