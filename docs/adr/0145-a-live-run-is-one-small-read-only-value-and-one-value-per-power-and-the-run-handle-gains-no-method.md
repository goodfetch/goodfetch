# A live Run is one small read-only value and one value per power, and the `Run` handle gains no method

What a live Run offers the engine's other modules becomes two tiers: one small read-only value that says which Run this is, and one value per power, each in the owner module ADR 0140 already names. From the day this ADR is adopted, `Run` gains no method. A spec placed right after the memory-first Run state (spec #2138) removes what is left of it outside the page crate, and the page crate's rework removes the rest. Decided by the owner in two grilling sessions on 2026-11-10 ([#2253](https://github.com/goodfetch/goodfetch/issues/2253), Q1, Q2, Q10 and Q11; [#2254](https://github.com/goodfetch/goodfetch/issues/2254), Q12). It builds on ADRs 0126, 0128 and 0140.

The Runs' registry is sound: one actor, one mailbox, one transition function with no wildcard arm. The handle beside it is where everything else is reached. `Run` is a cloneable record of ten fields, and its `impl` runs for about 1,230 lines. Whoever holds one can write the run file, charge Usage, set progress, write the Run trace, draw entropy, read settings and call the registry. A text count on 2026-11-10 finds 223 parameters or fields named `run` of type `Run` outside tests: 109 in the page crate, 71 in the crawl crate, 32 in the fetch tier's and 11 in the engine.

- **No rule is broken by the number of methods.** ADR 0126 refuses an alarm on it. What does not hold is the file's own `Size:` sentence, which says the file stays whole "so every change to a Run's state passes through one loop": the methods of `Run` never pass through that loop.
  > Note (correction, 2026-11-10, from an independent review of the publication): "never" is inexact. Of the 63 methods of that `impl`, 60 never pass through that loop, and three send it a command: the end, `uses`, and a charge refused because the Usage ledger cannot be stored (`crates/goodfetch-engine/src/runs.rs:1945-1956`, `:2542-2547` and `:2520-2529`, read at `197a8c18e`). The decision is unchanged: the `Size:` sentence still does not hold for the other 60.
- **ADR 0140 already names an owner module for each service:** `ledger::charging` for a Usage charge (E3), `run_trace` "as one writer per Run" (E5), the `run_state` writer (F2). The handle is where they are reached, not where they live.
- **Three adopted tickets are about to rebuild three of them:** [#1888](https://github.com/goodfetch/goodfetch/issues/1888) (charging), [#2243](https://github.com/goodfetch/goodfetch/issues/2243) to [#2167](https://github.com/goodfetch/goodfetch/issues/2167) (the crawl's Run state) and [#1890](https://github.com/goodfetch/goodfetch/issues/1890) (the Run trace's writer). With no rule, each would land its new methods on `Run`.
- **The one precedent built a second handle.** Commit `823766a89` (2026-11-07) moved the engine's own services out of `Runs` into `EngineContext`, a record of ten fields reached as `CallContext::engine()`. It is not the model here.
- **A start's disk work runs inside the registry's loop.** The loop's `answer` awaits `start`, which reads the run file, takes the writer's lock with retries and checks the ledger, so every other Run's command waits behind it.

## Decision

### Two tiers

- **The Run's identity** is one small read-only value behind one `Arc`: the Run's ID, its folder, its clock, its resolved settings, its stop signal, its metrics, its `tracing` span and its task owner. Cloning it is one reference count.
- **Each power is a value of its own,** in the owner module ADR 0140 names for it, or in a module of its own where ADR 0140 names none: the run file's writer, charging, the Run trace, progress, Secrets and the data key, the Run report's warnings, the end.
- **A function takes the identity and the powers it uses,** and nothing else of the Run.

> Note (agent, accepted by the owner on 2026-11-10), from the same review: the two lists do not cover everything `Run` hands out. Read at `197a8c18e` with the language server, the handle has 64 methods, one of them in `runs/outcome.rs`. 32 are the identity's or a listed power's, and 5 hand out what the engine's context already holds. Ten more follow from the rule above with no choice left, and are placed so:
>
> - a point in time is read from the identity's clock, and an outcome record from its Run ID;
> - the hold of a Human pause is progress's;
> - why a Run did nothing is the Run report's, written with its warnings;
> - what a Run used so far is a function over charging and the run file's writer;
> - asking a Run to stop, and the watches that ask it at a timeout or a storage failure, stay the Runs' registry's; a charge that a Limit refuses asks it too, as today.
>
> The other 17 hand out nine things that neither list names: the Run's share of the Resource governor, entropy, the Politeness a Run shares, its robots.txt copies, where a crawl attempt's writes are staged, the storage of bodies and of kept files, the signal that a Run was let finish, the record that a Run uses another Product's entry, and its records in the Run index. Each needs a new power, a new part of the identity or another home. That is the owner's to decide, and nothing is chosen here: [#2255](https://github.com/goodfetch/goodfetch/issues/2255) holds the question with its options, and [#2299](https://github.com/goodfetch/goodfetch/issues/2299) builds the answer.

> Note (owner's decisions, 2026-11-10, [#2255](https://github.com/goodfetch/goodfetch/issues/2255), in the grilling of decision G-10 of the consolidated review, answered "Yes, all eleven" and "Yes to both"): the nine are placed.
>
> - **Five more powers,** each in its owner module, so the powers are twelve: the Run's share of the Resource governor, with the governor's last sample (the governor's module, ADR 0140's E4); entropy, a value that forks in the order it forks today (ADR 0059); the Politeness a Run shares, opened on the first ask (the Politeness module, E6); the Run's own robots.txt copies, the one holder of "one fetch per origin while the Run lives" (the robots.txt module); and the Run's files, which stores a page's body and keeps a file for the program (the Run files' module).
> - **The end carries the signal that a Run was let finish.** The driver that ends a Run is the one that waits to be let finish, and the registry alone sets the signal. The identity's list stays as it is.
> - **Four are no value of the Run.** Where a crawl attempt's writes are staged arrives with the contribution that registers the crawl's driver, asked with the Run's ID and the attempt's crawl request ID and lease epoch; no contribution is added. The robots.txt copy a crawl shares is held by the crawl's own record of its Run. The record that a Run uses another Product's entry is a command of the registry's handle that takes the Run's ID. The two records a Run leaves in the Run index are functions of the Run index's module over the identity and the engine's context, never a command to the registry's loop.
> - **The registry alone sets a Run's stop signal.** A power that must ask for a stop, charging at a Limit's refusal and the Politeness a Run shares at a storage failure of its file, is built with a value of the registry's that asks.
> - **The registry's loop awaits no file.** [#2258](https://github.com/goodfetch/goodfetch/issues/2258) moves every disk wait of the loop beside it, not only a start's and a resume's.
>
> No Shared path's owner module or Cost contract changes (ADR 0140). [#2299](https://github.com/goodfetch/goodfetch/issues/2299) builds the placements.

### `Run` gains no method

- From this ADR's adoption, no change adds a method to `Run`.
- A ticket that rebuilds one of a Run's services leaves it in its owner module. #1888, #2243 and #1890 each gain one sentence saying so; none of their decided designs changes.

### How a handler receives them

- **A handler's function names what it takes,** as the crawl's `links(params, _context, crawls, tiers)` does today.
- **Each domain's record of a live Run** (a crawl's, a Session's) holds the identity and the powers that domain uses, in place of a `Run`.
- **No accessor of `CallContext` hands out a Run's power.** `CallContext` keeps what belongs to a call.

### The Runs' registry

- **The loop keeps the decision:** the Lifecycle's table (ADR 0128) and a mark that a Run is starting.
- **A start's and a resume's disk work runs in a task beside the loop,** which reports back by a command, as an end, a discard and a Checkpoint already do.
- **A lookup stays a message.** The registry still answers which Run.

### When

- **This ADR is adopted now,** before #1888 starts.
- **Two steps follow spec #2138 in `docs/agents/rollout-order.md`, before spec #1838's CDP tickets,** in this order: the spec of the `Run` handle ([#2255](https://github.com/goodfetch/goodfetch/issues/2255)), then the page crate's rework ([#2256](https://github.com/goodfetch/goodfetch/issues/2256)). The owner confirmed on 2026-11-10 that this amends the order adopted on 2026-11-01 and 2026-11-06.
- **The spec of the `Run` handle** defines the identity and the powers, moves the start's disk work, and edits the parameters and fields of type `Run` outside `goodfetch-page`.
- **The page crate's rework** edits those inside `goodfetch-page`, once, as it gives each owner its own task owner. `Run` as a handle is removed only when that rework has landed.

### What does not change

The Operation catalog, every operation's params and result, the run file's format, the Lifecycle's states and events, and the composition's list of contributions.

## Considered Options

Each reason is the one put to the owner before the answer ([#2253](https://github.com/goodfetch/goodfetch/issues/2253), section 7).

- **One spec after spec #2138, with no rule before it:** not chosen. #1888 would first rebuild charging inside `impl Run`, and the spec would then move it.
- **The whole change before spec #2138:** not chosen. It would narrow the admission, the Leases and the charges that spec #2138 removes.
- **No narrowing:** not chosen. No ADR requires it, and each feature spec then adds to the handle. Of the 30 feature specs' bodies, 24 mention the Run trace, 21 the Run report, 17 progress and 16 Usage: a count of mentions, not of what each would add.
- **Six values of equal rank:** not chosen. Every function names each thing it reads, so the page crate's signatures would take three or four parameters in place of one (an estimate, not counted per function). The delivery loop's `drive` already has nine inputs under a lint that allows seven.
- **One handle seen through traits:** not chosen. The signatures look narrow, but the value still carries everything, a clone stays heavy and the `Size:` sentence stays untrue.
- **Typed extraction at a handler's registration,** as axum's `State` and `FromRef` do: not chosen. What an operation touches could be read in one list and tested, but every params type must say which Run it names, and a second lookup path is added beside the one each domain already holds.
- **Accessors on `CallContext`:** not chosen. One more wide object, which is what this ADR ends.
- **A published directory of live Runs, read without a message:** not chosen. No round trip per call, but a second way to reach the registry's state, with a race of its own at a Run's end (a design argument, not a defect read in code).
- **No change to the registry's loop:** not chosen. A start or a resume goes on holding the other Runs' calls back during its disk work.

## Consequences

- **`docs/agents/rollout-order.md` gains two steps** after spec #2138; spec #1838's CDP tickets and specs #2139, #2164 and #2194 run after them.
- **The Run's identity keeps its task owner.** A Session's and a browser's owner are made as its children ([ADR 0146](0146-a-session-a-page-and-a-browser-each-have-one-owner-a-page-has-one-reader-of-its-events-and-one-module-of-the-page-crate-names-cdp.md)); what ends is page code starting tasks on the Run itself.
  > Note (owner's decision, 2026-11-10, [#2255](https://github.com/goodfetch/goodfetch/issues/2255), in the grilling of decision G-10 of the consolidated review): "read-only" means that no holder changes what the identity says of the Run. It does not mean without authority. Its task owner starts work that stops with the Run: a task, blocking work, an analysis, an actor, a child owner. The identity hands out that starting side alone. Cancelling, waiting for and stopping the Run's own owner stay the Runs' registry's, and the type the identity hands out has none of the three (`crates/goodfetch-engine/src/tasks.rs:260-293`, read at `094c96b2d`, where one type has both sides; the Run's own owner is stopped in one place, as the Run ends, `crates/goodfetch-engine/src/runs.rs:6401`). A child is a full owner, which its Session, Page or browser stops (ADR 0146). The identity also holds the Run's folder as a path: writing in it is the Run's files power's by its signature, not by enforcement. [#2257](https://github.com/goodfetch/goodfetch/issues/2257) builds it.
- **`runs`' list in `modules.toml` shrinks,** and each module split out of it gets its own.
- **A start that runs beside the loop can overlap another command.** The spec's model-based test of the registry holds what the loop held by awaiting: one start per Run ID, a stop or a discard that arrives while a Run is starting, a drain that waits for the starts in flight.
- **Not decided here:** how the engine stops naming Foxhound items is ADR 0099's rule, applied by [#2231](https://github.com/goodfetch/goodfetch/issues/2231).

## Existing code

Read at `21fe3ef3d`.

- `Run` is a cloneable handle of ten fields. `crates/goodfetch-engine/src/runs.rs:1235#Run`
- Its `impl` runs from line 1314 to line 2548. `crates/goodfetch-engine/src/runs.rs:1314-2548#Run`
- What every handle of a live Run shares holds its charging, its Secrets, its data key, its progress and its Provenance log. `crates/goodfetch-engine/src/runs.rs:1249#Shared`
- The file's `Size:` sentence gives the one loop as its reason to stay whole. `crates/goodfetch-engine/src/runs.rs:45-46`
- The registry is one actor whose loop answers each command in turn. `crates/goodfetch-engine/src/runs.rs:4092#Registry::run`
- The loop awaits a start before it takes the next command. `crates/goodfetch-engine/src/runs.rs:4130-4133`
- A start reads the run file, takes the writer's lock with retries and checks the ledger. `crates/goodfetch-engine/src/runs.rs:4935` `crates/goodfetch-engine/src/runs.rs:4952` `crates/goodfetch-engine/src/runs.rs:4982`
- The Lifecycle has one transition function. `crates/goodfetch-engine/src/runs/lifecycle.rs:92#apply`
- The engine's own services are one record of ten fields. `crates/goodfetch-engine/src/engine_context.rs:33#EngineContext`
- A crawl handler's function names what it takes. `crates/foxhound/goodfetch-crawl/src/enqueue/links.rs:136-141#links`
