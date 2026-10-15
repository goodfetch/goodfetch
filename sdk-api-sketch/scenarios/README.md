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
| 08 | [Extract with AI inside a crawl, learning once per page type](08-ai-in-crawl-handler.md) | Crawlee 3.18 and Crawlee for Python 1.11 | Both |
| 09 | [Add an AI step to an existing Playwright program](09-ai-in-playwright-program.md) | Stagehand 3.7 | Collie |
| 10 | [Test a crawl offline](10-offline-crawl-tests.md) | Scrapy 2.19 | Foxhound |
| 11 | [Reach a goal from one instruction, with limits and a verdict](11-agent-goal.md) | Browser Use 0.13 | Collie |
| 12 | [Run a crawl inside a web service](12-crawl-in-a-service.md) | Scrapy 2.19 | Foxhound |

Each task statement says what the developer wants, never which Goodfetch call does it, so a ticket that decides against a shape still has to answer the task another way. If [Close the pain coverage matrix](https://github.com/goodfetch/automan/issues/485) finds a pain answered in the SDK API that no scenario shows, it adds that pain to the closest scenario first, and adds a new scenario only if none fits. It added none: each pain no scenario shows is a way down (G8), a removed or deprecated name, a page or Session feature no task on toscrape.com needs, or a Hook no golden task needs, so a snippet shows it and the matrix row says why.

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
| Leader, TypeScript | `scenarios/leaders/NN-slug.<library>.ts`, or `scenarios/leaders/stagehand3/` for the programs on Stagehand 3.7.3 (08, 09), each folder with its own `package.json` |
| Leader, Python | `scenarios/leaders/sNN_slug_<library>.py`, importable by module name so a test imports the spider it tests; one environment per leader, pinned in `check.sh` |

`check.sh` type-checks the leaders' programs against their real packages: tsc 7.0.2 in each TypeScript folder, and pyright in standard mode in one `uv` environment per Python leader. Browsers are not downloaded and nothing is run.

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

## Comparison

Filled by [API sketch: type-checked stubs and golden scenarios](https://github.com/goodfetch/goodfetch/issues/483). Each cell is **concepts · lines · failure points typed/generic/silent**, from each description's Measures table, where the counted names, the setup commands, every failure point's score and the verdict are. Scenario 09 counts only the added step's lines; scenario 10 counts the test files only.

| # | Leader | Goodfetch TypeScript | Goodfetch Python | Goodfetch Rust |
|---|---|---|---|---|
| [01](01-dependent-form.md) | Playwright (TypeScript): 24 · 37 · 1/5/0<br>Playwright (Python): 26 · 38 · 1/5/0 | 27 · 34 · 6/0/0 | 26 · 44 · 6/0/0 | 39 · 57 · 6/0/0 |
| [02](02-typed-extract.md) | Stagehand (TypeScript): 41 · 78 · 0/4/2 | 28 · 43 · 6/0/0 | 28 · 56 · 6/0/0 | 36 · 63 · 6/0/0 |
| [03](03-replay-ai-steps.md) | Stagehand (TypeScript): 29 · 52 · 0/5/1 | 28 · 27 · 6/0/0 | 28 · 37 · 6/0/0 | 40 · 64 · 6/0/0 |
| [04](04-login-human-pause.md) | Playwright (TypeScript): 21 · 42 · 1/2/3<br>Playwright (Python): 22 · 39 · 1/2/3 | 41 · 59 · 6/0/0 | 39 · 58 · 6/0/0 | 52 · 86 · 6/0/0 |
| [05](05-two-page-crawl.md) | Crawlee (TypeScript): 31 · 69 · 3/2/3<br>Scrapy (Python): 36 · 91 · 0/5/3 | 45 · 69 · 8/0/0 | 48 · 96 · 8/0/0 | 55 · 166 · 8/0/0 |
| [06](06-mixed-fetch-crawl.md) | Crawlee (TypeScript): 35 · 100 · 1/5/0<br>scrapy-playwright (Python): 42 · 78 · 1/5/0 | 49 · 76 · 6/0/0 | 48 · 97 · 6/0/0 | 62 · 123 · 6/0/0 |
| [07](07-resume-and-retry.md) | Scrapy (Python): 46 · 77 · 0/3/3<br>Crawlee (TypeScript): 35 · 43 · 0/4/2 | 38 · 42 · 6/0/0 | 39 · 63 · 6/0/0 | 44 · 106 · 6/0/0 |
| [08](08-ai-in-crawl-handler.md) | Crawlee with Stagehand (TypeScript): 34 · 102 · 0/3/3<br>Crawlee for Python with PydanticAI: 32 · 98 · 1/4/1 | 44 · 51 · 6/0/0 | 46 · 66 · 6/0/0 | 54 · 126 · 6/0/0 |
| [09](09-ai-in-playwright-program.md) | Stagehand 3 with Playwright (TypeScript): 13 · 24 · 1/2/2 | 10 · 17 · 5/0/0 | 13 · 24 · 5/0/0 | 21 · 40 · 5/0/0 |
| [10](10-offline-crawl-tests.md) | Scrapy (Python): 16 · 34 · 0/3/1 | 31 · 48 · 5/0/0 | 32 · 62 · 5/0/0 | 39 · 89 · 5/0/0 |
| [11](11-agent-goal.md) | Browser Use (Python): 29 · 64 · 1/6/0 | 35 · 37 · 7/0/0 | 39 · 64 · 7/0/0 | 43 · 70 · 7/0/0 |
| [12](12-crawl-in-a-service.md) | Scrapy (Python): 29 · 65 · 6/0/0 | 41 · 83 · 6/0/0 | 34 · 65 · 6/0/0 | 41 · 105 · 6/0/0 |

- **Failure points.** Of the 73 failure points, Goodfetch's TypeScript, Python and Rust programs each score 73 typed; the leaders' 18 programs score most of theirs generic, and 27 silent between them. 03's first, a wrong step that still ends on an author page, passes the task statement's coarse Success check, so every 03 program, Stagehand's included, also checks the tag's second page and the first quote's author after its step: a wrong step is then a typed `success_check_failed` in Goodfetch's programs (ADR 0091) and the program's own `Error` in Stagehand's. The generic one first scored, a failed Export write in 05, is typed since [ADR 0116](https://github.com/goodfetch/goodfetch/blob/main/docs/adr/0116-a-saved-file-is-written-whole-or-not-at-all-and-a-failed-write-says-whether-its-path-or-its-storage-failed.md) ([#527](https://github.com/goodfetch/goodfetch/issues/527)). The silent one first scored, 09's two drivers acting on a borrowed page at once, which G1 does not accept, is typed since [ADR 0114](https://github.com/goodfetch/goodfetch/blob/main/docs/adr/0114-a-program-and-collie-take-turns-on-a-page-other-code-drives-and-an-ai-verb-fails-when-a-navigation-it-did-not-start-replaces-its-document.md) ([#533](https://github.com/goodfetch/goodfetch/issues/533)).
- **Lines.** Against a TypeScript leader, Goodfetch's TypeScript is as short or shorter in eight of nine scenarios, the exception being 04, where it states the Saved identity, the Secret binding and the expired-login outcome that Playwright's program leaves out. Against a Python leader, Goodfetch's Python is shorter in 07 and 08, level in 11 and 12 and longer in the rest, mostly from typed error handling that ruff's 88 columns split. Rust, with no scored leader, is the longest everywhere.
- **Concepts.** Goodfetch names fewer concepts than its leader in 02, 03 and 09, where the leader needs a price table, a file of resolved actions or a schema check of its own, and in Python's 07 (Scrapy's `JOBDIR` and signals); as many in Python's 01; and more in the others: its programs read typed outcomes, reasons, Verdicts and fixes that the leaders' programs leave as exception text or the program's own code (G1, G6), and they close what they open (G4).
- **Setup.** Goodfetch needs one package install per language, browsers being fetched on first use (ADRs 0024, 0057), where Playwright, Stagehand and scrapy-playwright add a browser install; a model scenario adds the model provider's key in every program.

## After the sketch

The golden scenarios outlive this branch. When the SDKs are built, `/to-spec` turns each into the SDK variants of a Journey (ADR 0063) and into the docs' tested sample for its task (ADRs 0030 and 0068), retargeted from toscrape.com to the Proving ground family named in its description. That keeps G8 ("no golden scenario needs a way down") and T checkable after 1.0, and the docs show the code the SDK API was judged on.
