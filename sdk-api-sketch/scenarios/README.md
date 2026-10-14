# Golden scenarios

A **golden scenario** is a complete program for a task developers commonly write. It exists in every SDK language, beside the leader's program for the same task, and the SDK API's shape is judged on it ([ADR 0077](https://github.com/goodfetch/automan/blob/main/docs/adr/0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md), G8 and T). It is not a Journey: a Journey is a test judged by the Proving ground's oracle, and a golden scenario is a design yardstick judged against a competitor's program.

[Golden scenarios and the pain coverage matrix](https://github.com/goodfetch/automan/issues/463) chose the twelve below and the rules on this page. [API sketch: type-checked stubs and golden scenarios](https://github.com/goodfetch/automan/issues/483) writes their programs and fills in their measures, and [Agent-writability check](https://github.com/goodfetch/automan/issues/484) gives each task statement to coding agents.

## The twelve

| # | Scenario | Leader | Product |
|---|---|---|---|
| 01 | [Fill a dependent form and read the results](01-dependent-form.md) | Playwright 1.64 | Collie |
| 02 | [Extract a typed object with a chosen model](02-typed-extract.md) | Stagehand 4.2 | Collie |
| 03 | [Repeat AI steps with no model call](03-replay-ai-steps.md) | Stagehand 4.2 | Collie |
| 04 | [Log in once with a person, reuse the login headless](04-login-human-pause.md) | Playwright 1.64 | Collie |
| 05 | [Crawl two page types into a Dataset](05-two-page-crawl.md) | Crawlee 3.18 and Scrapy 2.19 | Foxhound |
| 06 | [Crawl with HTTP and a browser only where needed](06-mixed-fetch-crawl.md) | Crawlee 3.18 and scrapy-playwright 0.0.48 | Foxhound |
| 07 | [Resume a killed crawl and retry its failures](07-resume-and-retry.md) | Scrapy 2.19 and Crawlee 3.18 | Foxhound |
| 08 | [Extract with AI inside a crawl, learning once per page type](08-ai-in-crawl-handler.md) | Crawlee 3.18 and Crawlee for Python 1.10 | Both |
| 09 | [Add an AI step to an existing Playwright program](09-ai-in-playwright-program.md) | Stagehand 3.7 | Collie |
| 10 | [Test a crawl offline](10-offline-crawl-tests.md) | Scrapy 2.19 | Foxhound |
| 11 | [Reach a goal from one instruction, with limits and a verdict](11-agent-goal.md) | Browser Use 0.13 | Collie |
| 12 | [Run a crawl inside a web service](12-crawl-in-a-service.md) | Scrapy 2.19 | Foxhound |

Each task statement says what the developer wants, never which Goodfetch call does it, so a ticket that decides against a shape still has to answer the task another way. If [Close the pain coverage matrix](https://github.com/goodfetch/automan/issues/485) finds a pain answered in the SDK API that no scenario shows, it adds that pain to the closest scenario first, and adds a new scenario only if none fits.

## A scenario's description

`NN-slug.md` holds, in this order:

1. **Task statement.** It is also, word for word, the prompt the Agent-writability check gives a coding agent, so the human comparison and the agent's attempt judge the same task.
2. **Target**: the pages it uses, on [toscrape.com](https://toscrape.com), the practice sites Zyte built for scraping and Scrapy's tutorial uses. They are stable, carry no robots.txt restrictions, and let anyone check a leader's program at no cost.
3. **Leader**: the library and version the program is set against, and which language each leader program is in.
4. **Pains**: the rows of [the pain coverage matrix](../pain-coverage.md) the scenario exercises.
5. **Failure points**: what really goes wrong in the task, written once and without reference to any language.
6. **Setup**: the commands each version needs before its program runs, written by #483 from each library's docs.
7. **Measures**: the table #483 fills in (see below).
8. **After the sketch**: the Journey the scenario becomes and the Proving ground family it is retargeted to.

## Programs

| Version | Path |
|---|---|
| TypeScript | `typescript/scenarios/NN-slug.ts` |
| Python | `python/scenarios/sNN_slug.py` |
| Rust | `rust/examples/sNN_slug.rs` |
| Leader | `scenarios/leaders/NN-slug.<library>.<ext>`, with the leader's own package manifest, type-checked by `check.sh` |

## The leader's program

- Written idiomatically from the leader's current official docs, taken from a docs example where one exists. It is never a strawman. The rule follows the benchmark's in [ADR 0025](https://github.com/goodfetch/automan/blob/main/docs/adr/0025-benchmark-per-verified-result-on-the-proving-ground.md): each competitor in its best documented configuration.
- The latest stable version, pinned in the description. A release candidate that changes the comparison, such as Crawlee 4.0, gets a one-line note.
- Type-checked against the real package (tsc, or pyright with the leader installed). It is not run.
- Compared language with language: Goodfetch's TypeScript with the leader's TypeScript, Goodfetch's Python with the leader's Python.
- The Rust version has no scored leader. No Rust library leads either family, and the research records the Rust crates' pains rather than a design to match. It is reviewed against the Rust API Guidelines gathered by [Research: multi-language SDK design guidelines from the leaders](https://github.com/goodfetch/automan/issues/464), and against Goodfetch's own TypeScript and Python versions for its concept count.

## Measures

| Measure | How it is counted |
|---|---|
| **Concepts introduced** | Distinct library names a reader must know (imported symbols, types, functions and methods, option keys, enum values), plus each lifecycle obligation (must close, must await, must dispose), counted once per program. It is Clarke's "working framework", the set of concepts a developer must hold at once ([Measuring API Usability](https://jacobfilipp.com/DrDobbs/articles/DDJ/2004/0405/0405j/0405j.htm)). |
| **Lines** | Non-blank, non-comment lines after the formatter `STYLE.md` names (oxfmt, ruff format, rustfmt), applied to the leader's program too, imports included. |
| **Setup** | The commands needed before the program runs (installs, browser downloads), counted apart from lines: it is where time to first result shows. |
| **Failure points** | Each of the scenario's failure points, scored per program: **typed** (an outcome or error that names the cause and what would fix it, G1 and G6), **generic** (an exception that has to be read as text) or **silent** (wrong data, a hang, or something that looks like success). |

**Verdict.** No numeric threshold. Where Goodfetch's program is worse than the leader's on a measure, the description gives the reason and cites a principle, for example "explicit close: G4". A **silent** failure point in a Goodfetch program is never accepted (G1).

The measures table of every scenario has this shape:

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Leader (language) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

The golden scenarios outlive this branch. When the SDKs are built, `/to-spec` turns each into the SDK variants of a Journey (ADR 0063) and into the docs' tested sample for its task (ADRs 0030 and 0068), retargeted from toscrape.com to the Proving ground family named in its description. That keeps G8 ("no golden scenario needs a way down") and T checkable after 1.0, and the docs show the code the SDK API was judged on.
