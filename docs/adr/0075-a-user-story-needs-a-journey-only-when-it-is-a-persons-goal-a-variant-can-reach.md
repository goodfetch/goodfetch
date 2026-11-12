# A user story needs a Journey only when it is a person's goal that a variant can reach

A user story gets `journey = true` only when it passes three tests in order: its actor is a person using a shipped surface, it states a whole goal rather than one part of one, and a Journey variant can reach its outcome with a person's own means. Every other story records which test it failed, and it is proven by tests lower in the pyramid, at least one of them above the unit level. One Journey covers every story whose goal it checks. Applied to areas 1 to 3, the rule flags 32 of 338 stories, covered by 20 Journeys, 10 of them new. Before, area 3's reading of the old rule flagged 129 of its 134 stories.

ADR 0072 sets `journey = true` on stories "whose actor is a person using Goodfetch". The spec of [Spec: Engine runtime (V1 area 3)](https://github.com/goodfetch/goodfetch/issues/213) applied that rule and flagged all but five of its 134 stories, since a developer is a person. That would make one Journey per lease rule, admission rule and governor rule, which is the inverted pyramid. Areas 1 and 2 read the same words more narrowly: area 1 flagged none of its 115 stories, and area 2 flagged 3 of 89. The owner asked for a test pyramid of unit, integration and end-to-end tests, with end-to-end tests "driven by a catalog of user journeys". Comparable projects choose what gets an end-to-end test in the same way:
- **Google** suggests "a 70/20/10 split: 70% unit tests, 20% integration tests, and 10% end-to-end tests", and asks "why write an end-to-end test when you can write a much smaller, more focused integration test that will detect the same bug?" ([Testing Blog, 2015](https://testing.googleblog.com/2015/04/just-say-no-to-more-end-to-end-tests.html)). *Software Engineering at Google* keeps larger tests for the case where "the overall behavior for specific user journeys is as intended", and says "a handful of large tests is preferable to an enormous one" ([chapter 14](https://abseil.io/resources/swe-book/html/ch14.html)).
- **Martin Fowler's site** (Ham Vocke, [The Practical Test Pyramid](https://martinfowler.com/articles/practical-test-pyramid.html)) says to "come up with user journeys that define the core value of your product", to keep end-to-end tests "to a bare minimum", and to "push your tests as far down the test pyramid as you can". It also says a test that a lower level already covers needn't be repeated higher up.
- **Kubernetes** admits a test to conformance only when "it limits itself to capabilities exposed via APIs" and "works without access to the public internet". It keeps out node- or platform-reliant features, optional ones, and "operational tools, rather than application-oriented" endpoints. Everything else stays in its wider e2e and integration suites ([conformance requirements](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/conformance-tests.md)). The bar is set by what a user's API exposes, not by who the user is.
- **Playwright** tests "user-visible behavior" and avoids "things which users will not typically use, see, or even know about" ([best practices](https://playwright.dev/docs/best-practices)). Its own repository keeps an `installation` suite that installs the packed packages with npm, yarn and pnpm and then uses them ([`tests/installation`](https://github.com/microsoft/playwright/tree/main/tests/installation), checked 2026-10-13). Installing and then using the product is the user journey it tests end to end.
- **Temporal** keeps one cross-SDK snippet per user-visible feature in [temporalio/features](https://github.com/temporalio/features) (129 entries under `features/<group>/`, checked 2026-10-13), run against a real server in each language. Behaviour inside one SDK stays in that SDK's own suites (unverified beyond the README's list of purposes).

Decided in [Which user stories need a Journey](https://github.com/goodfetch/goodfetch/issues/314), with the grilling answered by the agent on the owner's delegation. It builds on ADRs 0058, 0063 and 0072, and the rules every story follows are in `TESTING.md`.

## Decision

### The flag rule

`/to-spec` applies three tests to each user story, in order. The first test the story fails sets `journey = false`, and that test's name goes in `not_journey`. A story that passes all three gets `journey = true`.

1. **`actor`: is the actor a person using a shipped surface?** That person may be a user, a developer writing code against an SDK or the CLI, an operator, a Recipe author or user, a Plugin author, a Console user, or the person behind an agent. The story fails when its actor is a contributor, a maintainer, the owner as releaser, an area, test support, a Client or Client author, a coding agent, Goodfetch Cloud, a site owner, or a reader of the benchmark.
   > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no suite, test layout, Journey surface, security boundary, Performance budget, Feature kind (`screen:`) or actor exists for the Console.
2. **`part`: does the story state a whole goal?** A goal is a task the person sets out to do or see, such as running, crawling, resuming, moving, watching, stopping, explaining, installing, exporting, or forming or restoring a group. A stop at a Limit the person set is a goal, and so is the failure a person must see while doing one. The story fails when its "I want" is only part of a goal:
   - one setting's effect;
   - one Error code, or a refusal of a wrong input, version, environment or configuration;
   - one event or field, or one operation's detail;
   - how the engine works: a file, a format, a lock, an epoch, a queue, an algorithm, a protocol message or a process tree.
3. **`reach`: can a Journey variant reach the outcome?** The outcome must show through a surface that ADR 0063 lists, in a variant run that ADR 0063 isolates. The variant starts from public steps on one host, with the Proving ground and services test support gives every variant. It may use only a person's own means: start, stop or kill a process, change a setting, run a command, add or remove a Daemon. The story fails when its outcome needs any of these:
   - a fault injected below the surface (the Simulation, crash VFS, LazyFS, Toxiproxy, memory pressure, clock control or hostile code);
   - an old released binary;
   - an external account or service, including object storage, a Politeness store and cloud hosts;
   - Kubernetes;
   - state outside the variant's Goodfetch home and processes (a service manager, a managed Operator policy);
   - an artifact or channel of the release pipeline.

   An older-format file from the Compatibility corpus is allowed, as ADR 0063 allows for upgrade Journeys.

- **The tests are read from the story's own words** ("I want" and "so that"). Two people applying them get the same result, and the fresh review checks each reason.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): no fresh review exists; this check is a review standard the `code-review` skill applies, and the owner sees its result before merging.
- **The reason is recorded.** In `features/stories/<area>.toml`, a story with `journey = false` carries `not_journey = "actor" | "part" | "reach"`, and a story with `journey = true` carries none. The Feature check fails on a missing, unknown or extra reason.
- **A new surface widens `reach`** only through an ADR that amends ADR 0063's list of surfaces.

### Evidence for every story

- **Every story, flagged or not, needs at least one declaring test above the unit level**: an integration test (the Simulation in `tests/it/simulation/` and the multi-process and crash tests of the binary, `kill -9` and LazyFS, included), a conformance case, a Client conformance case or a Journey. In-process crash-VFS, model-based and loom tests are unit tests of their module, as `TESTING.md` places them. An exception of ADR 0072 also meets this rule. `xtask features` reads a test's level from the declaration's path, so no runner reports it:
  - unit or integration by `TESTING.md`'s Levels and layout;
  - conformance for a case under `conformance/`, `conformance/clients/` included;
  - Journey for a test in a `journeys` folder (`tests/it/journeys/` in Rust, `tests/journeys/` in TypeScript and Python).
  _Why:_ a story is a promise across a seam, and a unit test proves one module. Matrix rows and generated Features keep ADR 0072's rule, where a unit test alone counts.
- **Which tests prove each kind of unflagged story:**

  | `not_journey` | Proven by |
  |---|---|
  | `actor` | the tests the story's contract needs: conformance or Client conformance cases for a Client or Goodfetch Cloud, area 1's `xtask` tests, release rehearsal and smoke runs, or `check` exceptions, for contributors and maintainers; the Proving ground's and the benchmark's own tests for its readers |
  | `part` | an integration test of the package that owns it, or the binary's tests, asserting the setting, Error code, event or mechanism, with conformance where several targets implement it |
  | `reach` | the test that can create the condition: the Simulation, crash and partition tests, multi-process tests, skew tests against released binaries, release rehearsal and deployment smoke runs, or a `live` or `manual` exception |

- **A flagged story needs those tests too** wherever its edge cases, Error codes and limits lie. The Journey proves the goal, as ADR 0063 requires.

### When a flagged story is due

- **Rule 5 of ADR 0072 is due at the later of two milestones:** the story's own, and the earliest milestone of any variant of a Journey that covers it. Until then the story counts as tested on its other declaring tests. A story the engine finishes in M1 but which people reach only through the SDKs (M2) is therefore due with its Journey at M2.
- **From then on, the story needs one proved variant of a Journey that covers it.** Its other variants count under rule 3 of ADR 0072.

### How Journeys take stories

- **A Journey covers a story only when one of its `[[expect]]` checks asserts that story's "so that" outcome**, as ADR 0072 requires of every declaration.
- **Extend or add.** A flagged story whose goal is an existing Journey's goal, or is the starting step of that goal (installing what the variant drives, as Playwright's installation suite installs before it uses), is added to that Journey's `covers`, with a check, and with a step when the goal needs one. Since the goal stays the same, the ID stays. A story with any other goal gets a new Journey. A Journey may cover any number of stories; there is no quota per area.
- **A Journey may also cover an unflagged story** whose outcome it asserts.

### The rule each later area follows

Each area's `/to-spec` applies the three tests to every story it writes, records `journey` and `not_journey` in its stories file, and then adds the Journeys its flagged stories need, extending existing ones first. Its `/to-tickets` names the variants each ticket makes `proved`, as ADR 0063 requires.

> Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): no skill writes repository files or ticket fields. The stories file (each story's flags and milestone) and the Journeys of flagged stories must exist before a test declares them; tickets add them like any other work.

## Considered Options

- **Keep ADR 0072's actor-only rule and plan large Journeys:** rejected. In area 3 it flags 129 stories. To cover them, either each Journey checks dozens of mechanisms, which goes against ADR 0063's "keep a Journey to its goal", or the catalog grows to hundreds of slow end-to-end tests. That inverts the pyramid that Google, Fowler and Kubernetes warn against.
- **Flag only stories whose outcome a person observes through a public surface** (the ticket's example): rejected as the whole rule. A lease, an admission rule or a power cut's survival also shows through a surface, as a missing duplicate or a resumed run. Without the `part` and `reach` tests, the rule still flags stories that lower tests prove better.
- **Flag by matrix row instead of story:** rejected. Rows already appear in Journeys' `covers`, but ADR 0072 asks for a flag per story, and rows group mechanisms with goals.
- **No flag, so `/to-spec` judges each story:** rejected. The ticket asks for a rule `/to-spec` applies mechanically, and judgment gives the drift seen between areas 1 and 3.
- **A quota of Journeys per area, or a share of 10 % as in Google's split:** rejected. Google gives its split as "a good first guess", and the count should follow the goals, not a target.
- **Let a unit test alone prove an unflagged story:** rejected. An unflagged person's story would then have no test of the composed package, and ADR 0072's other rules wouldn't notice.
- **A Journey for each story reached only through an install channel or the release pipeline:** rejected. Their end-to-end proof is area 1's release rehearsal, deployment smoke runs and ADR 0069's installed-package runs, which use the release's own artifacts. A Journey drives the commit's packages, so the one install goal it can reach (installing the packages into a fresh project) goes into J-002's SDK variants.

## Consequences

- **Amends ADR 0072** (notes there record it): `/to-spec` sets the flag by this ADR's three tests, not by the actor alone; every story also needs a declaring test above the unit level; rule 5 is due at the later of the story's milestone and the earliest milestone of a covering variant; `features/stories/<area>.toml` gains `not_journey`; and the causes of an untested Feature gain "no test above the unit level" and "no covering Journey".
- **Amends ADR 0063** (a note there records it): each area's `/to-spec` adds Journeys for its stories with `journey = true`, not for every user story.
- **Adds to ADR 0038**, so ADR 0037's amendment rule carries it to every area's spec: each `/to-spec` applies the three tests and records `not_journey`. ADR 0037's area table lists this ADR in the rows of areas 1, 2 and 3.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): no skill writes repository files or ticket fields. The stories file (each story's flags and milestone) and the Journeys of flagged stories must exist before a test declares them; tickets add them like any other work.
- **The "Test infrastructure" spec builds:** the `not_journey` field in the stories type and its schema; the level each declaration has, read from its path; the two new causes in the report; the due milestone of rule 5. It also writes the Journey files below and the extra `covers`, checks and steps of the existing ones.
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/goodfetch/goodfetch/issues/130)) through the delta pass:
  - `journey = true` for stories 1, 2 and 3 (`npm install`, `pip install`, `cargo add`) and 36 (`goodfetch upgrade-files`). Every other story is `false`. The `not_journey` reason is `actor` for stories of contributors, maintainers, the owner and coding agents; `reach` for stories that need release artifacts, channels, registries, signing, the docs site or a CI job; and `part` for the rest.
  - Stories 1 to 3 are covered by J-002's `sdk-typescript`, `sdk-python` and `sdk-rust` variants, whose starting step becomes installing Goodfetch from the commit's packages into a fresh project, with a check that the installed package runs; J-002's goal is unchanged. Story 36 is covered by the new upgrade Journey below.
  - The spec's Journeys paragraph no longer says this area adds no Journey and that every story is `journey = false`. It says this area adds no Journey of its own, and that four of its stories are covered by J-002 and the upgrade Journey.
- **Changes area 2's spec** ([Spec: Proving ground and conformance (V1 area 2)](https://github.com/goodfetch/goodfetch/issues/152)): no flag changes. Stories 51, 52 and 77 stay `true`, covered by its one planned Journey. Every other story takes `not_journey = "actor"`: its actor is an evaluator or another reader of the benchmark (stories 13 and 14's AI users included), a competitor, its maintainer or its user, a contributor, a maintainer, an operator of CI, the owner, an area, test support, a Journey's author or a surface's test runner. Story 41 keeps its `manual` exception.
- **Changes area 3's spec** ([Spec: Engine runtime (V1 area 3)](https://github.com/goodfetch/goodfetch/issues/213)) through the delta pass:
  - **`journey = true`** for 25 stories: 1, 2, 13, 18, 23, 24, 34, 35, 67, 71, 73, 76, 79, 89, 96, 98, 99, 106, 111, 113, 115, 116, 119, 125 and 128.
  - **`not_journey = "actor"`:** 3, 30, 43, 80 and 109.
  - **`not_journey = "reach"`:** 21 (a service manager), 77 (a managed Operator policy), 83 (a Politeness store), 101 (the ops listener is no Journey surface) and 114 (Kubernetes). These state whole goals; every other story that also needs a fault or a service fails `part` first.
  - **`not_journey = "part"`:** every other story (99 stories).
  - **Existing Journeys gain `covers`:** J-001 gains `story:3-18`, with a check of the idle stop after an idle time shortened through its setting (ADR 0059); J-005 gains `story:3-23`; J-006 gains `story:3-67`, `story:3-71`, `story:3-76` and `story:3-79`, and its first step takes the monthly install Limit that `goodfetch init` offers; J-007 gains `story:3-24`; J-009 gains `story:3-89`; J-012 gains `story:3-34`.
  - **The two M4 Journeys the spec already plans** cover `story:3-115` (a person Limit across a Daemon group, #258) and `story:3-116` and `story:3-119` (a Partitioned crawl read as one run, #248).
  - **Ten new Journeys**, numbered from `journeys/catalog.toml` when the "Test infrastructure" spec writes them:

    | Title | Surfaces (milestone) | Proving ground | Covers |
    |---|---|---|---|
    | Run a one-off SDK script on an Attached engine, then two scripts that share the Daemon, both seen in one Console | `sdk-rust`, `sdk-typescript`, `sdk-python` (M2), with a step pinned to `console` | `catalog` | `story:3-1`, `story:3-2`, `row:RT-19` |
    | Pause a Crawl, restart the Daemon and resume it | `cli` (M1); the SDKs (M2) | `catalog` | `story:3-35`, `row:RT-11` |
    | Run the Daemon in the foreground, drain it with `SIGTERM` and resume its runs when it starts again; its checks are the foreground Daemon's own outcome (the configured address, JSON logs, no runtime file, no idle stop), not Docker, Kubernetes or systemd | `cli` (M1) | `catalog` | `story:3-98`, `story:3-99` |
    | Be told once each at 50 %, 80 % and 100 % of a Limit | `cli`, `console` (M4) | `catalog` | `story:3-73` |
    | Move to a new major with `goodfetch upgrade-files`, leaving the previous major's files unchanged | `cli` (M4), starting from the Compatibility corpus | none | `story:1-36`, `story:3-96` |
    | Form a Daemon group and remove a Group member while its runs go on | `cli` (M4) | `catalog` | `story:3-106`, `story:3-111` |
    | Restore a Daemon group from its backup after the Lead Daemon is stopped | `cli` (M4) | `catalog` | `story:3-113` |
    | Run a TypeScript Script Hook in a Crawl | `cli` (M4); the SDKs (M4) | `catalog` | `story:3-125` |
    | Feed a keep-alive run new work until its input closes | the SDKs (M2) | `catalog` | `story:3-128`, `row:RT-12` |
    | Call an Exposed Daemon from an edge function over WebSocket | `sdk-typescript` (M4) | `catalog` | `story:3-13` |

    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.

    > Numbered on 2026-10-19 in [#348](https://github.com/goodfetch/goodfetch/issues/348), in area 3's spec's order: the ten new Journeys are J-013 to J-022 in this table's order, the person Limit across a Daemon group J-023 and the Partitioned crawl J-024. By ADR 0094's rule, J-014, J-020, J-021 and J-024 are Foxhound's (a Crawl, a keep-alive Crawl's Frontier, a Partitioned crawl) and the other eight Collie's.
  - **Due milestones later than the story's own:** stories 1, 2 and 128 (M1) are due with their Journeys at M2, since their only variants are on the SDKs; story 96 (M1) is due at M4, since the upgrade Journey's only variant is `cli` at M4. The other flagged stories are due at their own milestones.
  - The spec's Journeys paragraph lists these Journeys. Its Further Notes replace "`journey = true` for every story whose actor is a person using Goodfetch" with the lists above.
- **The size of the change:** across areas 1 to 3, 32 stories are flagged instead of 132 (4 in area 1, 3 in area 2, 25 in area 3). They are covered by 20 Journeys: J-002, the 6 existing Journeys that gain area 3 stories, area 2's planned Journey, the 2 planned M4 Journeys and the 10 new ones.
- **No new glossary term.** The flag and its reasons are fields, not domain terms.

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): the `part` row's integration tests include the Simulation, and the binary's tests include each Product's executable's crash tests (`kill -9`, LazyFS); several `part` stories (9, 22, 26, 72, 112) are proven only there.
