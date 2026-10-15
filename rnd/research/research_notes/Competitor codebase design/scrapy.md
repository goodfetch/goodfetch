# Scrapy (scrapy/scrapy): how the codebase is designed (as of 2026-10-16)

Evidence gathered on 2026-10-16 from a read-only clone at commit `fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c` (committed 2026-10-15), Scrapy 2.19.0 ([scrapy/VERSION](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/VERSION)). Sources: the code itself, the repository's config, CI and docs, and the issue tracker (read with `gh`). Nothing from the clone was run.

How to read the tags:
- An untagged claim is stated in, or counted from, the cited file.
- `[inferred: basis]` marks my own deduction.
- "Maintainers say" marks a statement by a Scrapy maintainer in an issue.
- Counts come from `tokei`, `grep`, or two AST scripts that parse the clone without importing it (import graph; `X`/`X_async` method pairs).

Permalink base: `https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/`. Below, every `path:Lx-Ly` link points to that commit.

## 1. Snapshot

| Item | Value |
|---|---|
| Repository | [scrapy/scrapy](https://github.com/scrapy/scrapy), BSD-3-Clause, Python ≥ 3.11, CPython and PyPy ([pyproject.toml:L9-L61](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/pyproject.toml#L9-L61)) |
| Commit / date | `fbfc89a`, 2026-10-15; version 2.19.0 |
| Languages (whole repo) | Python 533 files, 103,705 lines (85,237 code); reStructuredText 81 files, 38,851 lines; 657 files, 148,151 lines in total |
| `scrapy/` (library) | 196 `.py` modules in 25 directories, 36,227 lines (29,944 code); 21 modules are underscore-private |
| `tests/` | 326 `.py` files, 66,362 lines (54,504 code); 173 `test_*.py`; 3,034 test functions; 293 `@pytest.mark.parametrize` |
| `docs/` | 59 `.rst`, 34,817 lines (Sphinx); 5 `.py` |
| Other | `tests_typing/` (3 mypy-testing files), `sep/` (20 historical enhancement proposals), `extras/` (release scripts), root `AGENTS.md` |
| Runtime dependencies | 22 core packages, including Twisted, aiohttp, aioftp, PyDispatcher, zope.interface, parsel, w3lib, itemadapter, queuelib, protego, `formerly` ([pyproject.toml:L9-L38](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/pyproject.toml#L9-L38)) |

Size per sub-package of `scrapy/` (files / lines): `core` 28 / 7,669 · `utils` 53 / 6,506 · top-level modules 25 / 5,444 · `extensions` 18 / 3,392 · `commands` 17 / 2,320 · `http` 12 / 2,186 · `downloadermiddlewares` 15 / 1,830 · `pipelines` 4 / 1,710 · `settings` 2 / 1,408 · `spiders` 4 / 1,356 · `spidermiddlewares` 9 / 959 · `linkextractors` 2 / 599 · `contracts` 2 / 595 · `selector` 2 / 143 · `loader` 1 / 106.

Module size distribution: median 134 lines, 90th percentile 420, 13 modules above 500. The ten largest:

| Lines | File |
|---|---|
| 1,345 | `scrapy/crawler.py` (Crawler plus six runner and process classes) |
| 1,102 | `scrapy/extensions/feedexport.py` (11 classes) |
| 1,046 | `scrapy/core/engine.py` |
| 926 | `scrapy/pipelines/files.py` |
| 903 | `scrapy/core/downloader/handlers/http11.py` |
| 736 | `scrapy/settings/__init__.py` |
| 680 | `scrapy/utils/defer.py` |
| 672 | `scrapy/settings/default_settings.py` |
| 583 | `scrapy/core/scraper.py` |
| 559 | `scrapy/core/scheduler.py` |

## 2. Paradigm

**Runtime and concurrency.** Everything runs on one event loop in one thread, and the loop can come from three runtime modes:
- Twisted with `AsyncioSelectorReactor`, the default; the test suite runs this mode by default with `--reactor=asyncio` ([pyproject.toml:L288-L291](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/pyproject.toml#L288-L291)).
- Twisted with a non-asyncio reactor.
- A "reactorless" pure-asyncio mode, `TWISTED_REACTOR_ENABLED=False`, added in 2.15.

`Crawler._apply_settings()` installs or verifies the reactor, or switches HTTP and FTP to the aiohttp and aioftp handlers when there is none ([crawler.py:L186-L294](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/crawler.py#L186-L294)). Reactorless mode installs an import hook that forbids importing `twisted.internet.reactor` ([utils/reactorless.py:L42-L68](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/reactorless.py#L42-L68)).

Scheduling is a pull loop. A `CallLaterOnce` "nextcall" plus a 5-second heartbeat drains the scheduler while `needs_backout()` is false. `needs_backout()` combines engine state, per-slot downloader limits and the scraper's in-flight response size ([core/engine.py:L140-L187](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L140-L187), [L498-L511](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L498-L511)).

**Style.** The code is class-based OO. Inheritance supplies base classes (`Spider`, `CrawlSpider`, four `MiddlewareManager` subclasses, `BaseStreamingDownloadHandler`). Composition happens at run time:
- the `Crawler` owns settings, signals, stats and the engine;
- the engine owns the scheduler, downloader and scraper;
- nearly every component class is picked by a dotted import path in settings.

Plugin hooks are duck-typed: `hasattr(mw, "process_request")` ([core/downloader/middleware.py:L53-L64](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/downloader/middleware.py#L53-L64)). Functional style appears only in streaming helpers: async generators and `iter_errback`.

**Public API style.** A user subclasses `Spider`:
- gives it a `name`;
- supplies `start_urls` or an `async def start()` generator;
- writes callbacks such as `parse(response)` that yield `Request`s or items (dicts, `Item`, dataclasses, attrs; normalized by the external `itemadapter`).

Per-spider settings go in `custom_settings` or `update_settings()` ([spiders/__init__.py:L30-L135](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/spiders/__init__.py#L30-L135), [L199-L313](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/spiders/__init__.py#L199-L313)). The top-level namespace re-exports only `Request`, `FormRequest`, `Item`, `Field`, `Selector` and `Spider` ([__init__.py:L10-L24](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/__init__.py#L10-L24)).

**Typing.** mypy runs with `strict = true` on the library, relaxed for `tests.*` ([pyproject.toml:L114-L124](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/pyproject.toml#L114-L124)). The package ships `py.typed`, and `tests_typing/` asserts inferred types of the public API with `reveal_type` ([tests_typing/test_spiders.mypy-testing](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/tests_typing/test_spiders.mypy-testing)). The data contracts stay loose, though:
- the library has 1,039 `Any` tokens, 108 `type: ignore` and 82 `cast(`;
- callbacks are typed `Callable[Concatenate[Response, ...], Any]` ([http/request/__init__.py:L44](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/http/request/__init__.py#L44));
- settings, `Request.meta`, stats and signal payloads are untyped (section 6).

## 3. Organization

**Top-level layout.**
- `scrapy/` holds the library.
- `tests/` is mostly flat: 173 `test_*.py` files mirror module paths ([docs/contributing.rst:L421-L430](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/docs/contributing.rst#L421-L430)). Mock HTTP, FTP, DNS and mitmproxy servers live in `tests/mockserver/`, and shared abstract suites in `tests/utils/bases/`.
- `docs/` is Sphinx narrative docs plus autodoc.
- `sep/` holds Trac-era proposals ("Most of them are in Trac Wiki format", [sep/README.rst](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/sep/README.rst)), several still marked "Draft" or "Document in progress" `[inferred: not a live design process]`.
- `extras/` holds release and CI-pin scripts.
- Root `conftest.py` selects the reactor for tests.

**Package boundaries.** The library is split by component kind:
- `core/`: engine, scheduler, scraper, `spidermw.py`, `downloader/` and `downloader/handlers/`;
- one package per plugin family: `downloadermiddlewares/`, `spidermiddlewares/`, `pipelines/`, `extensions/`;
- shared types and utilities: `http/`, `spiders/`, `settings/`, `commands/`, `utils/`;
- 25 top-level modules, among them `crawler.py`, `middleware.py`, `signals.py`, `exceptions.py`, `pqueues.py` and `dupefilters.py`.

The built-in plugins sit in packages named after the extension point they plug into, which makes them easy to find. Names are inconsistent, though:
- `core/spidermw.py` sits beside the `spidermiddlewares/` package;
- some names are concatenated (`statscollectors`, `pqueues`, `squeues`);
- four unrelated classes are called `Slot`: engine `_Slot` ([core/engine.py:L140](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L140)), scraper `Slot` ([core/scraper.py:L78](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/scraper.py#L78)), downloader `Slot` ([core/downloader/__init__.py:L46](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/downloader/__init__.py#L46)), and `FeedSlot`.

**Dependency direction (measured).** I parsed every module and classified each `scrapy.*` import between first-level sub-packages or modules as one of three kinds:
- runtime, at module level: 569;
- `TYPE_CHECKING`-only: 242;
- function-local: 8.

The runtime graph has one strongly connected component of 17 nodes: `addons, core, crawler, extension, http, item, linkextractors, middleware, pipelines, resolver, responsetypes, selector, settings, signalmanager, spiderloader, spiders, utils`. Only `exceptions` is a true leaf. The main cause is that `utils` is not a leaf:
- [utils/test.py:L16](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/test.py#L16) imports `scrapy.crawler`;
- [utils/spider.py:L7](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/spider.py#L7) imports `scrapy.spiders`;
- `utils/response.py`, `utils/iterators.py` and `utils/conf.py` import `http`, `selector` and `settings`.

A package-level cycle is not the same thing as a module-level import cycle, but real module cycles exist too:
- ruff carries "Circular import workarounds" ignores for `spiders/__init__.py` and `linkextractors/__init__.py` ([pyproject.toml:L500-L503](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/pyproject.toml#L500-L503)), whose subclasses are imported at the bottom of the file ([spiders/__init__.py:L333-L336](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/spiders/__init__.py#L333-L336));
- 12 "circular import" comments mark function-local imports, for example [utils/reactor.py:L64-L88](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/reactor.py#L64-L88);
- pylint `cyclic-import` is disabled ([pyproject.toml:L231](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/pyproject.toml#L231)).

The heavy `TYPE_CHECKING` use, enforced by ruff's `TC` rules, is what keeps most type-only cycles from breaking at import time.

**Public and internal API.** The rule is the underscore prefix: "Methods or functions that start with a single underscore (`_`) are private" ([docs/versioning.rst:L42-L43](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/docs/versioning.rst#L42-L43)). `AGENTS.md` makes it the default for new code ([AGENTS.md:L26-L28](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/AGENTS.md#L26-L28)).

Because the old public `utils` surface grew too large, 12 of its 37 public modules are now import-time-deprecated shims that re-export a private twin, for example `utils/sitemap.py` → `utils/_sitemap.py` ([utils/sitemap.py:L5-L12](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/sitemap.py#L5-L12)). Two more signs of loose boundaries:
- only 21 modules declare `__all__`;
- 41 test files still import private names (48 import lines), although `AGENTS.md` says "Test through public APIs".

**File size discipline.** Small modules are the norm (median 134 lines), but nothing enforces a limit:
- pylint `too-many-lines` is disabled ([pyproject.toml:L254](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/pyproject.toml#L254));
- branch and statement limits are suppressed inline on the engine's `close_spider_async()`, about 160 lines long ([core/engine.py:L875](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L875)).

## 4. Design patterns catalog

| Pattern | Where | Problem it solves | Assessment |
|---|---|---|---|
| Composition root and service locator | `Crawler` builds stats, log formatter, fingerprinter, extensions and engine, then freezes settings ([crawler.py:L186-L249](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/crawler.py#L186-L249)); `get_extension()` and the other `get_*()` methods find components by class ([L413-L497](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/crawler.py#L413-L497)) | One object to pass to every component; ordered set-up | **Mixed.** Clear root, but components reach through it into internals: `self.crawler.engine.downloader._delay = …` ([extensions/throttle.py:L45](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/extensions/throttle.py#L45)); 18 `crawler.engine.` reach-throughs outside the engine |
| Uniform factory method | `from_crawler` classmethod plus `build_from_crawler()`, typed with a ParamSpec `Protocol` ([utils/misc.py:L154-L204](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/misc.py#L154-L204)); 72 `from_crawler` definitions; `from_settings` survives only for spider loaders ([spiderloader.py:L19-L23](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/spiderloader.py#L19-L23)) | Every plugin gets settings, signals and stats the same way | **Good**, with one gap: the factory is optional (`hasattr`), so the constructor contract is implicit, and the core does not always use it: `load_object(STATS_CLASS)(self)` ([crawler.py:L196](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/crawler.py#L196)), `downloader_cls(crawler)` is called directly, and `Scraper(crawler)` is hard-coded ([core/engine.py:L236-L248](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L236-L248)) |
| Plugin registry by dotted path, with ordering numbers | `load_object()` ([utils/misc.py:L64-L96](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/misc.py#L64-L96)); `*_BASE` dicts such as `DOWNLOADER_MIDDLEWARES_BASE` (50…900) ([default_settings.py:L341-L359](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/settings/default_settings.py#L341-L359)) merged with the user dict, normalized by import path, `None` disables ([settings/__init__.py:L338-L385](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/settings/__init__.py#L338-L385), [utils/conf.py:L21-L64](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/conf.py#L21-L64)) | Users insert, reorder or disable built-ins without subclassing | **Mixed.** Very flexible and the same for every family; but a global magic-number space, and typos surface only at run time |
| Chain of Responsibility | Downloader middlewares: `process_request` forward with short-circuit; `process_response` and `process_exception` reversed via `appendleft` ([core/downloader/middleware.py:L53-L163](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/downloader/middleware.py#L53-L163)) | Composable request and response processing (retry, redirect, cookies, cache) | **Good** core idea. Return types are checked at run time (`_InvalidOutput`) because hooks are duck-typed |
| Template Method | `BaseStreamingDownloadHandler` owns limits, data loss, maxsize and body reading; subclasses fill hooks (`_make_request`, `_iter_body_chunks`, `_is_timeout_exception`) ([handlers/_base_streaming.py:L66-L180](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/downloader/handlers/_base_streaming.py#L66-L180)); `BaseSpiderMiddleware.get_processed_request/item` ([spidermiddlewares/base.py:L15-L110](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/spidermiddlewares/base.py#L15-L110)) | aiohttp and httpx handlers share one behavior; middleware authors write per-element logic while the framework owns sync and async iteration | **Good.** The best DRY tool in the codebase |
| Observer (signals) | 26 sentinel signals `engine_started = object()` … ([signals.py:L8-L32](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/signals.py#L8-L32)); `SignalManager` over PyDispatcher's process-global registry, keyed by sender ([signalmanager.py:L17-L40](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/signalmanager.py#L17-L40)) | Extensions react to lifecycle and request events without coupling | **Mixed.** Strong decoupling; payloads are untyped `**kwargs`; three send implementations (section 5); the dependency is called outdated in [#5557](https://github.com/scrapy/scrapy/issues/5557), open since 2022 |
| State machine | `EngineState` enum plus `_STATE_TRANSITIONS` table and `_transition_to()` ([core/engine.py:L82-L137](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L82-L137), [L296-L307](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L296-L307)) | Start and stop races, hangs on early close ([#6916](https://github.com/scrapy/scrapy/issues/6916)) | **Good but late and soft.** Added in the unreleased version (`versionadded:: VERSION`); an invalid transition only logs a warning and still happens |
| Strategy | Scheduler, dupefilter, priority queues, fingerprinter, stats and log formatter are chosen by setting ([core/engine.py:L254-L261](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L254-L261), [crawler.py:L196-L204](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/crawler.py#L196-L204)) | Swap crawl-order and persistence policies | **Good**, but interface checking is inconsistent: the scheduler uses a metaclass `__subclasscheck__` ([core/scheduler.py:L29-L45](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/scheduler.py#L29-L45)); others use `typing.Protocol` (11 classes) |
| Null Object | `DummyStatsCollector` ([statscollectors.py:L159](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/statscollectors.py#L159)); `MemoryStatsCollector` overrides the `_persist_stats` hook (L137-L156) | Disabling stats without `if stats:` at every call site | **Good** |
| Late-bound typed attribute (descriptor) | `_LateAttribute` makes `crawler.engine` and `crawler.stats` non-optional in types and raises `RuntimeError` if read before the crawl starts ([crawler.py:L76-L134](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/crawler.py#L76-L134)) | Avoids `Optional` narrowing everywhere | **Good** local fix for a two-phase init problem the design created |
| Lazy registry by key | `DownloadHandlers` maps URL scheme or name to a handler, instantiates lazily, remembers `NotConfigured` ([handlers/__init__.py:L61-L150](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/downloader/handlers/__init__.py#L61-L150)) | Optional dependencies (boto, h2, httpx) cost nothing until used | **Good** |
| Adapter (Deferred ↔ coroutine) | `deferred_from_coro`, `maybe_deferred_to_future`, `ensure_awaitable` branch on `is_asyncio_available()` at run time ([utils/defer.py:L489-L680](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/defer.py#L489-L680), [utils/asyncio.py:L54-L104](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/asyncio.py#L54-L104)) | A gradual Twisted → asyncio migration without a flag day | **Mixed.** It made the migration possible, but the adapter sits in the hot path and 21 call sites branch on the mode |
| Decorator plus docstring DSL | Spider contracts: `@url`, `@returns` and `@scrapes` lines parsed from callback docstrings; callbacks wrapped with pre and post hooks feeding a `unittest.TestResult` ([contracts/__init__.py:L96-L122](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/contracts/__init__.py#L96-L122), [L185-L233](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/contracts/__init__.py#L185-L233)) | Cheap smoke tests for spiders (`scrapy check`) | **Mixed.** Clever, but the registry is a class-level mutable `ClassVar` dict written by every instance ([L166](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/contracts/__init__.py#L166), [L183](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/contracts/__init__.py#L183)) |
| Opt-in future behavior (feature flag) | `DOWNLOADER_MIDDLEWARE_RESPONSE_EXCEPTIONS`, plus a warning that names only the third-party `process_exception` methods affected ([core/downloader/middleware.py:L165-L198](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/downloader/middleware.py#L165-L198)) | Changing pipeline semantics without breaking plugins | **Good.** Targeted and actionable |
| Compatibility shims | Module `__getattr__` ([default_settings.py:L659-L670](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/settings/default_settings.py#L659-L670)); signature probing `argument_is_required(method, "spider")` (9 call sites, [utils/deprecate.py:L135-L154](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/deprecate.py#L135-L154)); `formerly.deprecated_class` ([L37-L77](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/deprecate.py#L37-L77)) | Removing API without breaking users | **Mixed.** Disciplined, but pervasive: 223 `ScrapyDeprecationWarning` sites in 66 of 196 modules |

## 5. Separation of concerns and DRY

**What works.**
- One loading loop serves four plugin families: `MiddlewareManager.from_crawler()` reads the ordered list, builds each component, skips `NotConfigured` and logs what is enabled ([middleware.py:L84-L111](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/middleware.py#L84-L111)). Downloader middlewares, spider middlewares, item pipelines and extensions only supply `_get_mwlist_from_settings` and `_add_middleware` ([extension.py:L18-L25](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/extension.py#L18-L25)).
- One merge function covers every `*_BASE` setting (`get_component_priority_dict_with_base`).
- One streaming base serves both new HTTP handlers.
- Stable, reusable subsystems were moved out into their own packages: `parsel`, `w3lib`, `itemadapter`, `itemloaders`, `queuelib`, `protego` and `formerly`, which is the deprecation helper proposed in [#2194](https://github.com/scrapy/scrapy/issues/2194).
- The roles of the core components are documented once ([docs/topics/architecture.rst:L21-L71](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/docs/topics/architecture.rst#L21-L71)).

**Where it fails.**
- **Dual async paths.** The AST scan finds 27 `X` / `X_async` method pairs, in engine, scraper, crawler, pipelines, middleware managers, signals and utilities. Most legacy halves are one-line deprecated wrappers (`deferred_from_coro(self.start_async())`, [core/engine.py:L309-L317](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L309-L317)). `Crawler.crawl()` and `crawl_async()` duplicate the whole body, because the Deferred version "Cannot use @deferred_f_from_coro_f because that relies on the reactor being installed already" ([crawler.py:L292-L360](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/crawler.py#L292-L360)). Inside "async" code, legacy Deferred chains with `addBoth` and `addErrback` lambdas remain ([core/engine.py:L517-L582](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L517-L582)).
- **Three signal-send implementations with different semantics** ([utils/signal.py:L36-L215](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/signal.py#L36-L215)):
  - sync: returns `Failure`s and always adds `StopDownload` to `dont_log`;
  - Deferred: returns `Failure`s, `dont_log` defaults to `None`;
  - asyncio: returns exceptions via `asyncio.gather`.

  `send_catch_log_async()` converts between them (L140-L162).
- **Mode-specific bugs.** Per-domain concurrency was not enforced under the asyncio reactor with zero delay ([#8256](https://github.com/scrapy/scrapy/issues/8256), closed 2026-10-01) `[inferred: an example of the dual paths diverging]`.
- **Repeated component-loading loops.** The same try / `load_object` / `build_from_crawler` / `except NotConfigured` / log sequence appears in four places:
  - [addons.py:L35-L55](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/addons.py#L35-L55);
  - [middleware.py:L89-L110](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/middleware.py#L89-L110);
  - [handlers/__init__.py:L113-L150](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/downloader/handlers/__init__.py#L113-L150);
  - `feedexport.py:L1029`.

  Extensions are not a chain, but they reuse `MiddlewareManager` by inheritance (`# pylint: disable=abstract-method`).
- **Two parallel storage hierarchies.**
  - Feed storages: `FeedStorageProtocol`, `File`/`Stdout`/`S3`/`GCS`/`FTPFeedStorage` ([extensions/feedexport.py:L167-L550](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/extensions/feedexport.py#L167-L550)).
  - Media stores: `FilesStoreProtocol`, `FS`/`S3`/`GCS`/`FTPFilesStore` ([pipelines/files.py:L86-L528](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/pipelines/files.py#L86-L528)).
  - Separate registries: `FEED_STORAGES_BASE` and `MEDIA_STORAGES_BASE` ([default_settings.py:L409-L417](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/settings/default_settings.py#L409-L417), [L518-L524](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/settings/default_settings.py#L518-L524)).
  - Only the FTP and boto helpers are shared. `S3FilesStore` takes its configuration as class attributes overwritten via `self.__dict__.update(config)` ([pipelines/files.py:L163-L210](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/pipelines/files.py#L163-L210)).
- **Five ways to declare an interface.**
  - deprecated `zope.interface` ([interfaces.py](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/interfaces.py); `_IFeedStorage`);
  - a metaclass duck check (scheduler);
  - `typing.Protocol` (handlers, spider loader, fingerprinter, queues, storages);
  - `ABC` (`BaseDownloadHandler`, `MiddlewareManager`);
  - bare `hasattr` (middleware hooks, add-on `update_settings`).

  `[inferred]` These layers record the history of Python typing idioms more than a design choice.
- **God objects.**
  - `ExecutionEngine` (1,046 lines) mixes lifecycle, the request pull loop, download dispatch, the item path for start items, and deprecation bridges for downloader and stats signatures ([core/engine.py:L237-L246](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L237-L246), [L994-L1005](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L994-L1005)).
  - `crawler.py` holds `Crawler` plus six runner and process classes with diamond inheritance (`CrawlerProcess(CrawlerProcessBase, CrawlerRunner)`, `AsyncCrawlerProcess(CrawlerProcessBase, AsyncCrawlerRunner)`, [L971](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/crawler.py#L971), [L1052](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/crawler.py#L1052)).
- **Leaky abstractions.**
  - The engine reads `scraper.slot` internals ([core/engine.py:L504-L510](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L504-L510), L587).
  - AutoThrottle writes the downloader's private `_delay`.
  - The shell drives a half-opened engine, and its own comment lists "Other architecture problems", including the use of private `Crawler` members ([shell.py:L75-L81](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/shell.py#L75-L81)). That comment names a `_create_engine()` that no longer exists anywhere in `scrapy/`, so the comment has drifted from the code.
- **Redundant `spider` parameters.** For years every hook received `spider` even where `crawler.spider` was available. Removing it takes a multi-release deprecation with run-time signature probing ([middleware.py:L116-L126](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/middleware.py#L116-L126); [#6763](https://github.com/scrapy/scrapy/issues/6763), [#6927](https://github.com/scrapy/scrapy/issues/6927)).
- **Stringly-typed contracts between components.** Components agree on plain strings, with no type checking:
  - 42 documented `Request.meta` special keys;
  - 346 string-keyed settings reads in 70 modules;
  - free-form stats keys such as `f"spider_exceptions/{type(exception).__name__}"` ([core/engine.py:L442-L445](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/engine.py#L442-L445)).

## 6. Contracts and extension points

**Extension points.** All of these are selected by settings:
- spiders;
- downloader middlewares and spider middlewares;
- item pipelines;
- extensions;
- download handlers, by scheme or by name through `DOWNLOAD_HANDLERS_BY_NAME`, typed by `DownloadHandlerProtocol` ([handlers/__init__.py:L41-L58](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/core/downloader/handlers/__init__.py#L41-L58));
- scheduler, dupefilter, priority queues and request fingerprinter;
- stats collector and log formatter;
- feed exporters and feed storages; media storages;
- spider loader;
- CLI commands, through the `scrapy.commands` entry-point group or `COMMANDS_MODULE` ([cmdline.py:L64-L86](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/cmdline.py#L64-L86));
- add-ons, which patch settings before components are built ([addons.py:L25-L72](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/addons.py#L25-L72));
- contracts (`SPIDER_CONTRACTS`).

The shared component contract is:
- an optional `from_crawler`;
- raise `NotConfigured` to stay disabled ([exceptions.py:L18-L27](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/exceptions.py#L18-L27));
- hooks may be sync or `async def`; returning a Deferred is deprecated, with a warning ([utils/defer.py:L655-L680](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/utils/defer.py#L655-L680)).

**Configuration.**
- `BaseSettings` stores `(value, priority)` pairs with six named levels: `default` 0, `command` 10, `addon` 15, `project` 20, `spider` 30, `cmdline` 40 ([settings/__init__.py:L31-L38](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/settings/__init__.py#L31-L38)).
- Dict settings get per-key priorities by nesting `BaseSettings` (L52-L76, L697-L719).
- Settings are frozen once the crawler is configured, and the overridden ones are logged ([crawler.py:L243-L249](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/crawler.py#L243-L249)).
- The defaults are 197 untyped module constants in one flat module with a long `__all__` list. Its header asks maintainers to keep them alphabetical and documented by hand ([default_settings.py:L1-L14](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/settings/default_settings.py#L1-L14)).
- Typing happens at read time (`getbool`, `getint`, …). I found no validation of setting names, so a misspelled key is silently ignored `[inferred: no unknown-key check in scrapy/settings or crawler.py]`.
- Docs declare 217 `.. setting::` entries, 42 `reqmeta`, 26 `signal` and 68 `stat`.

**Error model.**
- The hierarchy is flat: about 19 classes, all directly under `Exception`, with no common Scrapy base ([exceptions.py](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/exceptions.py)).
- Several exceptions are control flow, not errors: `IgnoreRequest`, `DropItem`, `CloseSpider`, `DontCloseSpider`, `NotConfigured`, `StopDownload`.
- Newer transport-neutral download errors (`DownloadTimeoutError`, `CannotResolveHostError`, `DownloadConnectionRefusedError`, `ResponseDataLossError`, `UnsupportedURLSchemeError`, [L122-L157](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/exceptions.py#L122-L157)) are raised the same way by the http11, aiohttp, httpx and aioftp handlers. This is a good normalization at the transport boundary.
- `ContractFail` subclasses `AssertionError`.
- Legacy paths carry Twisted `Failure` objects while async paths carry exceptions.

**Logging, stats and observability.**
- Logging is stdlib `logging`, with `extra={"spider": …}` for context.
- `LogFormatter` is a strategy that returns `TypedDict` log records ([logformatter.py:L28-L193](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/logformatter.py#L28-L193)).
- `StatsCollector` is a string-keyed `MutableMapping`.
- Extensions cover LogStats, PeriodicLog, MemoryUsage, EventLoopLagMonitor and the Telnet console.
- New in 2.19, enabled by default: `RemoteControl`, a localhost aiohttp server with a bearer token. Its `/execute` endpoint runs arbitrary Python inside the crawl, and it backs the "Scrapy MCP server" ([extensions/remote_control.py:L42-L97](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/extensions/remote_control.py#L42-L97), [default_settings.py:L556](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/settings/default_settings.py#L556), [docs/topics/agents.rst](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/docs/topics/agents.rst)). The agent integration is an eval channel, not a typed protocol.

**Deprecation policy and machinery.**
- Policy: "We aim to maintain support for deprecated Scrapy features for at least 1 year" ([docs/versioning.rst:L52-L65](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/docs/versioning.rst#L52-L65); [published page](https://docs.scrapy.org/en/latest/versioning.html#deprecation-policy)). Contributors must check the 12 months before removing anything ([docs/contributing.rst:L210-L212](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/docs/contributing.rst#L210-L212)).
- Docs for deprecated features are removed immediately and the `deprecated` directive is banned. `versionadded` and `versionchanged` use a `VERSION` placeholder and are pruned after 3 years ([docs/contributing.rst:L335-L343](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/docs/contributing.rst#L335-L343); [AGENTS.md:L60-L64](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/AGENTS.md#L60-L64)).
- `ScrapyDeprecationWarning` subclasses `Warning`, not `DeprecationWarning`, so users see it ([exceptions.py:L193-L196](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/exceptions.py#L193-L196)).
- The test suite runs with `filterwarnings = ["error", …]` ([pyproject.toml:L306-L308](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/pyproject.toml#L306-L308)).
- Version numbers: *A.B.C*, where B releases "may break backward compatibility" (versioning.rst:L10-L20).

## 7. Quality machinery

**Formatting and linting.** pre-commit runs ruff-check `--fix`, ruff-format, blacken-docs, sphinx-lint, the `sphinx-scrapy` sync hook and zizmor, which audits GitHub Actions ([.pre-commit-config.yaml](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/.pre-commit-config.yaml)).

Ruff `extend-select` takes 33 rule families: `A, ASYNC, B, C4, COM, D, E4/E7/E9, F, FA, FLY, FURB, I, ISC, LOG, PERF, PGH, PIE, PL, PT, PTH, PYI, Q, RET, RSE, RUF, S, SIM, SLOT, T10, TC, TID, UP, W, YTT`. About 30 rules are ignored, mainly missing-docstring `D1xx` rules and `PLR0913`/`PLR0917` argument counts ([pyproject.toml:L350-L486](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/pyproject.toml#L350-L486)). One architectural rule is enforced by the linter: `banned-module-level-imports = ["twisted.internet.reactor", …]` (L488-L495).

pylint uses `fail-on = "I"` with `useless-suppression` enabled, so a stale `# pylint: disable` fails CI. Even so, it disables about 50 checks, including `cyclic-import`, `duplicate-code`, `too-many-lines`, `protected-access` and `no-member`, and keeps a bucket labelled "Ones that we may want to address" (L211-L286).

Inline suppressions in `scrapy/`: 110 `# noqa`, 31 `pylint: disable`, 108 `type: ignore`, 81 `pragma: no cover`.

**Type checking.**
- `mypy --strict` runs on `scrapy` and `tests` with pinned stubs, on Python 3.11 ([tox.ini:L69-L101](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/tox.ini#L69-L101)).
- `mypy-tests` runs `pytest-mypy-testing` over `tests_typing/`.
- Twisted is exempted with `untyped_calls_exclude`; `scrapy.interfaces` has `ignore_errors`.

**Tests.**
- pytest with xdist, timeout, rerunfailures and sybil (doctests in docs); `xfail_strict = true` ([pyproject.toml:L288-L305](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/pyproject.toml#L288-L305)). Markers gate tests by runtime mode (`only_asyncio`, `requires_reactor`) and by optional dependency.
- Integration-heavy by policy: "We usually prefer end-to-end tests" and "Do not add tests that use as much mocking as possible" ([docs/contributing.rst:L147-L150](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/docs/contributing.rst#L147-L150)). Local mock servers make this possible.
- Shared conformance suites: `TestHttpBase`, `TestHttpsBase`, `TestHttpProxyBase` and others ([tests/utils/bases/download_handlers_http.py](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/tests/utils/bases/download_handlers_http.py), 1,959 lines). Each handler reruns them through a mixin ([tests/test_downloader_handler_aiohttp.py:L30-L106](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/tests/test_downloader_handler_aiohttp.py#L30-L106)).
- Benchmarks under `tests/benchmarks` run in CodSpeed workflows.
- Coverage is collected (branch, including subprocesses), but there is no threshold: no `fail_under`, and the codecov project status is off ([codecov.yml](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/codecov.yml)).

**CI** ([.github/workflows/ci.yml](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/.github/workflows/ci.yml)):
- A gate job runs pre-commit, then pylint, mypy, mypy-tests, docs, docs-tests and twinecheck. The test jobs declare `needs: [pre-commit, checks]` (L173, L252, L299), so a lint failure "costs a couple of minutes of CI instead of dozens of test jobs" (L22-L25).
- The test matrix crosses:
  - Python 3.11 → 3.15-rc and PyPy 3.11;
  - the three runtime modes (`py`, `default-reactor`, `no-reactor`);
  - `min-*` envs pinned to the oldest supported dependencies, plus `extra-deps` and `botocore` variants;
  - Ubuntu, Windows and macOS.
- Each failing check prints a job step with the local command to reproduce it.
- Other workflows: publish with PyPI trusted publishing (`id-token: write`, [publish.yml:L40-L51](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/.github/workflows/publish.yml#L40-L51)), CI-pin upgrades, VCS-dependency tests, spam-PR closing, and Dependabot.

**Contribution rules.**
- [docs/contributing.rst](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/docs/contributing.rst):
  - every patch needs tests;
  - docstrings only "when they add something significant";
  - API reference lives in docstrings via autodoc, and private APIs must be listed in `coverage_ignore_pyobjects` so that `docs-coverage` passes.
- [AGENTS.md](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/AGENTS.md) adds rules agents tend to break:
  - "Do not add settings, parameters or abstractions for needs nobody has yet";
  - cover every added line;
  - `parametrize` instead of loops in tests;
  - build components with `build_from_crawler()` in tests;
  - squash-merge, no force-push after review;
  - two-sentence PR descriptions.
- Design documents: only the stale `sep/`. Live design discussion happens in issues, for example [#6916](https://github.com/scrapy/scrapy/issues/6916) and [#8111](https://github.com/scrapy/scrapy/issues/8111).

## 8. Pain points

What maintainers say:
- **Twisted and asyncio dual support is the main cost.** Opening [#8111](https://github.com/scrapy/scrapy/issues/8111) (2026-09-03, open), maintainer wRAR lists the pros of dropping non-asyncio reactors: it "removes a lot of compatibility code and poorly tested alternative code paths from Scrapy, simplifies the docs". His benchmark table in the thread shows the default reactor 1.5–1.7× slower on item-heavy scenarios. Gallaecio agrees, through the "regular deprecation treatment".
- **The engine lifecycle was fragile.** [#6916](https://github.com/scrapy/scrapy/issues/6916) (2025): start and stop are "complicated multi-stage processes" with several entry points. Early shutdown leads to "the spider process hanging" or "(cascades of) unhandled AttributeErrors", and "existing tests often depend on timing and aren't reliable". The `scrapy.shell` code is "dirty, calling private methods directly". The current `EngineState` machine is the response.
- **Initialization order leaks.** [#7812](https://github.com/scrapy/scrapy/issues/7812) (2026-07, open): `is_asyncio_available()` cannot be used in an extension's `__init__`, because extensions are created in `Crawler._apply_settings()` before the loop runs, contrary to the docs.
- **Performance regressions from the migration.** [#7158](https://github.com/scrapy/scrapy/issues/7158): 2.13 was 2–3× slower on an item-heavy spider; a maintainer bisected it to PRs #6729 and #6852.
- **Old dependencies.** PyDispatcher is "badly outdated" ([#5557](https://github.com/scrapy/scrapy/issues/5557), open since 2022). `zope.interface` is being replaced by Protocols ([interfaces.py:L9-L13](https://github.com/scrapy/scrapy/blob/fbfc89ae7b72531a97fb4ef6d65ef3af392aa45c/scrapy/interfaces.py#L9-L13)).
- **API surface debt.** The `spider` argument is removed piecemeal: "this will require breaking changes or a deprecation period" ([#6927](https://github.com/scrapy/scrapy/issues/6927)). The repository's own config labels lint debt "Ones that we may want to address" and "Circular import workarounds".

My inferences:
- The core's complexity is mostly migration scaffolding: three runtime modes, 27 method pairs, runtime mode checks, Deferred chains inside async code.
- The extension model's flexibility (dotted paths, duck typing, string keys) pushes errors to run time and keeps types loose (`Any`).
- `utils` acts as a dumping ground that creates package-level cycles. The team is now paying to privatize it module by module.

## 9. Scorecard

| Dimension | Score (1–5) | Justification |
|---|---|---|
| Layering and dependency direction | 2 | 17-node package-level import SCC; `utils` imports `crawler` and `spiders`; circular-import ignores in ruff; `cyclic-import` disabled |
| Module and file size discipline | 3 | Median 134 lines, p90 420; but `crawler.py`, `feedexport.py` and `engine.py` exceed 1,000 lines, and size and complexity lints are disabled or suppressed |
| DRY (no language ports; Python only) | 2 | 27 `X`/`X_async` pairs, three signal senders, two storage hierarchies, four copies of the loading loop; partly offset by `MiddlewareManager` and the streaming template |
| Separation of concerns | 3 | Clear documented component roles and per-family packages; but the engine is a god object, components touch private fields, and the shell uses private APIs |
| Extension model | 4 | Very complete and uniform (`from_crawler`, priority dicts, `NotConfigured`, signals, add-ons, entry points); weakened by duck typing and five interface styles |
| Contracts and types | 3 | mypy strict, Protocols, type-inference tests; but 1,039 `Any`, untyped settings, `meta`, stats and signal payloads |
| Error model | 2 | Flat, no base class, exceptions used as control flow, `Failure` and exception split; download errors are now normalized across transports |
| Test architecture | 4 | 3,034 tests, local mock servers, shared conformance suites, warnings as errors, min-deps and three runtime modes; but flat 173-file directory, a 1,959-line base file, timing-dependent tests per maintainers |
| Tooling and CI enforcement | 4 | Broad ruff, pylint failing on stale suppressions, strict mypy, doc build, coverage and doctests, zizmor, gated CI; no coverage threshold, about 50 pylint checks off |
| In-code documentation | 4 | Rich docstrings feeding autodoc, `versionadded` discipline, comments that explain why, an AGENTS.md; stale comments exist (`shell.py` names a removed method), and the SEPs are abandoned |

## 10. Lessons for Goodfetch

- **Adopt: one construction contract for every component, made mandatory.** Scrapy's `from_crawler` and `build_from_crawler` uniformity is its best property. Because it is optional and duck-typed, the core itself bypasses it (`Scraper(crawler)`, `downloader_cls(crawler)`). Goodfetch could use a single trait such as `fn build(ctx: &Context) -> Result<Self, Disabled>`, where `Disabled` plays the role of `NotConfigured` as a typed outcome rather than an exception.
- **Adopt: layered settings with provenance, frozen at start, and a log of overrides.**
  - Keep from Scrapy: the six-level priority model, `None` to disable a built-in, the overridden-settings log.
  - Avoid: 197 untyped constants read by string 346 times, with no unknown-key check.
  - Goodfetch's settings, including capabilities, should be typed structs in `goodfetch-protocol`, with a schema emitted by `xtask codegen`, and should reject unknown keys.
- **Avoid: more than one async model in the core.** Scrapy's three runtime modes produce 27 duplicated method pairs, 21 run-time mode branches, bugs specific to one mode ([#8256](https://github.com/scrapy/scrapy/issues/8256)) and regressions ([#7158](https://github.com/scrapy/scrapy/issues/7158)). Maintainers call the alternative paths "poorly tested" ([#8111](https://github.com/scrapy/scrapy/issues/8111)). Goodfetch should be Tokio-only in the core; blocking or sync facades belong in generated SDKs, never as twin methods.
- **Adopt from day one: an explicit lifecycle state machine with a transition table**, ideally typestate or at least hard errors. Scrapy retrofitted `EngineState` after years of hangs on early close ([#6916](https://github.com/scrapy/scrapy/issues/6916)), and it still only warns on invalid transitions.
- **Adopt: shared conformance test suites per extension trait**, run against every implementation, as in `tests/utils/bases/download_handlers_http.py`. For Goodfetch this means one suite per transport or storage trait, and one protocol suite run against every SDK language and both CLI and MCP.
- **Adopt: per-element hooks so the framework owns iteration, async and backpressure** (`BaseSpiderMiddleware.get_processed_request/item`). Plugins that never see the stream cannot break streaming or force sync and async twins.
- **Avoid: a `utils` grab-bag and layering by convention.** Scrapy's `utils` sits inside a 17-node import cycle, and 12 public `utils` modules are now deprecated shims. With Rust crates, enforce direction by crate boundaries, keep helper crates leaf-only, and default to `pub(crate)`.
- **Avoid: stringly-typed signals, stats and request metadata, and eval-style agent access.**
  - Scrapy has 26 `object()` signals with `**kwargs` payloads and three send implementations, 42 string `meta` keys, and free-form stats names.
  - Its MCP integration runs arbitrary Python through `/execute`.
  - Goodfetch should define events, metrics and per-request options as typed protocol items, generated into the SDKs, with one emit path, and expose MCP through typed tools.
- **Adopt: normalized transport errors; avoid a flat, base-less, control-flow exception set.** Scrapy's handlers all raise the same `DownloadTimeoutError` family, which is good. `DropItem`, `IgnoreRequest` and `CloseSpider` as exceptions, with no common base and `Failure` objects alongside exceptions, are not. In Rust: one error enum per layer, with control outcomes (drop, ignore, close) as return variants.
- **Adopt later, design now: the deprecation toolkit.** Scrapy has a policy of at least one year, warnings as errors in tests, a visible warning category, opt-in future flags with warnings that name the affected third-party code, and private-twin shims. Goodfetch is pre-release and needs none of it yet. The contract-versioning hooks (ADRs 0030, 0041, 0047) should still exist before the first publish, so that later removals don't need run-time signature probing like Scrapy's `argument_is_required`.
