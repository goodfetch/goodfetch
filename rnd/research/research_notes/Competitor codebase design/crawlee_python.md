# Crawlee for Python: codebase design notes

Repo `apify/crawlee-python`, commit [`508f544`](https://github.com/apify/crawlee-python/tree/508f544f7f0260b4f26b806d50e84592538f8db6) (2026-10-15, version 1.11.0). Read statically (grep, `tokei`, an AST import scan run from a scratch script); nothing in the clone was executed. Below, `P/` stands for `https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/`, and every `P/...` link is a permalink with line numbers.

## 1. Snapshot

- **What it is:** a Python port of the JS Crawlee, built for web scraping and browser automation. It ships as one PyPI distribution (`crawlee`) with optional extras for each capability ([P/pyproject.toml#L52-L101](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/pyproject.toml#L52-L101)). It requires Python 3.10 or later, and CI tests 3.10 to 3.15.
- **Languages:** Python for the library. The docs site is Docusaurus (MDX, JS, CSS). There are 4 Lua scripts (156 lines) for the Redis RQ client. The 377k lines of JSON are frozen API snapshots under `website/versioned_docs`.
- **Size (tokei, Python):**

| Directory | Files | Lines | Code |
|---|---|---|---|
| `src/crawlee` (without `project_template`) | 186 | 33,973 | 27,363 |
| `tests/unit` | 107 | 31,868 | 23,126 |
| `tests/e2e` | 4 | 330 | 250 |
| `docs` (code examples, plus 67 MDX files) | 227 | 8,589 | 5,275 |
| `website` (mostly versioned copies of the docs examples) | 350 | 13,022 | 7,855 |

- **Size per subpackage** (`src/crawlee/*`, Python files / lines): `crawlers` 57 / 8,489; `storage_clients` 33 / 8,133; `_utils` 28 / 3,898; `browsers` 11 / 2,041; `request_loaders` 7 / 1,715; `storages` 7 / 1,515; `http_clients` 5 / 1,473; `_autoscaling` 5 / 1,139; `sessions` 5 / 912; `statistics` 5 / 735; `events` 4 / 574; `fingerprint_suite` 6 / 522; `otel` 2 / 155. The 11 top-level modules add 2,672 lines, including `_types.py` (839), `_request.py` (436), `_cli.py` (301) and `proxy_configuration.py` (268).
- **Module count:** 29 packages (directories with an `__init__.py`) and 186 modules. Per-file size: median 114 lines, p90 436, 12 files over 500 lines, 58 files of 50 lines or fewer.
- **10 largest source files:** `crawlers/_basic/_basic_crawler.py` 1,898; `storage_clients/_sql/_request_queue_client.py` 855; `storage_clients/_file_system/_request_queue_client.py` 846; `_types.py` 839; `_utils/sitemap.py` 802; `crawlers/_pydantic_ai/_selector_extractor.py` 726; `request_loaders/_throttling_request_manager.py` 723; `crawlers/_playwright/_playwright_crawler.py` 651; `_utils/html.py` 640; `storage_clients/_redis/_request_queue_client.py` 620.
- **Tests:** 1,297 `def test_` functions. Test code is about 0.85 times the size of the source code.

## 2. Paradigm

- **Runtime:** a single `asyncio` event loop.
  - `AutoscaledPool` takes three callbacks (`run_task_function`, `is_task_ready_function`, `is_finished_function`) and scales the number of concurrent worker tasks from `Snapshotter` CPU, memory and event-loop samples ([P/src/crawlee/_autoscaling/autoscaled_pool.py#L38-L104](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/_autoscaling/autoscaled_pool.py#L38-L104)).
  - Blocking work goes through `asyncio.to_thread` one call at a time: parsers, sync event listeners, and about 40 call sites in the file-system storage clients (for example [P/src/crawlee/crawlers/_beautifulsoup/_beautifulsoup_parser.py#L27-L34](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_beautifulsoup/_beautifulsoup_parser.py#L27-L34)). There is no process pool.
- **Style:** heavy object-oriented inheritance, with injected collaborators.
  - Crawlers form a generic class tree. Contexts are frozen dataclasses that get extended by subclassing.
  - Collaborators (`HttpClient`, `StorageClient`, `SessionPool`, `BrowserPool`, `RequestManager`) are passed to the constructor, or else resolved from a global service locator.
  - The functional parts: pipeline middlewares are async generators, and the context helpers (`push_data`, `enqueue_links`, ...) are closures typed by `Protocol`s ([P/src/crawlee/_types.py#L327-L625](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/_types.py#L327-L625)).
- **Public API style:** you instantiate a crawler, register handlers with decorators on `crawler.router` (`default_handler`, `handler(label=...)`, `use` for middleware), add `pre_navigation_hook` and `error_handler` callbacks, and then call `await crawler.run(urls)`. Handlers receive a typed context, such as `BeautifulSoupCrawlingContext` with `.soup`, `.push_data` and `.enqueue_links` ([P/src/crawlee/router.py#L19-L127](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/router.py#L19-L127)).
- **Typing:**
  - Pydantic v2 at data boundaries, with 33 `BaseModel`s and `pydantic-settings` for configuration.
  - `TypedDict` + `Unpack` to forward kwargs through constructors.
  - PEP 696 `TypeVar` defaults, so `BasicCrawler[TCrawlingContext, TStatisticsState]` works without type arguments.
  - Type checking uses Astral's pre-1.0 `ty` with its default rule set. The only rule raised is `unused-ignore-comment = "error"` ([P/pyproject.toml#L291-L312](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/pyproject.toml#L291-L312)).
  - The source contains 183 `Any` mentions, 34 `cast(...)` calls and 9 `ty: ignore` comments.

## 3. Organization

- **Top level:**
  - `src/crawlee` is the only package.
  - `tests/unit` mirrors the source tree, and `tests/e2e` tests the project templates.
  - `docs/` holds MDX guides plus `code_examples` that are linted and type-checked.
  - `website/` is Docusaurus with frozen `versioned_docs`.
  - `scripts/` has two updaters (Playwright version, public suffix list).
  - The cookiecutter template for the `crawlee create` CLI lives inside the package, at `src/crawlee/project_template`.
- **Public vs internal:**
  - An `_`-prefix marks private modules and packages. Each public package's `__init__.py` re-exports the API with an explicit `__all__` ([P/src/crawlee/__init__.py#L1-L21](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/__init__.py#L1-L21), [P/src/crawlee/crawlers/__init__.py#L1-L157](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/__init__.py#L1-L157)).
  - A no-op `@docs_group(...)` decorator, applied 140 times, tags what the API reference renders ([P/src/crawlee/_utils/docs.py#L7-L47](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/_utils/docs.py#L7-L47)).
  - Inconsistencies:
    - `_autoscaling` is private but documented as public (`@docs_group('Autoscaling')`), and its module files have no underscore (`autoscaled_pool.py`), unlike everywhere else. The same goes for `otel/crawler_instrumentor.py`.
    - There are 17 `py.typed` markers where one suffices.
    - `storage_clients/_file_system/_utils.py` is empty.
- **"Private" is not private:**
  - The agent guide lists private modules that the Apify SDK imports and subclasses, which makes them a de-facto interface ([P/.rules.md#L61-L70](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/.rules.md#L61-L70)).
  - The contributing guide counts logger names and log message wording as breaking changes ([P/CONTRIBUTING.md#L127](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/CONTRIBUTING.md#L127)). Logger names come from private module paths, for example `crawlee._autoscaling.autoscaled_pool` in [P/docs/examples/fill_and_submit_web_form.mdx#L105](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/docs/examples/fill_and_submit_web_form.mdx#L105).
- **Dependency direction** (AST scan of module-level runtime imports between top-level subpackages):
  - The intended shape holds: `crawlers` depends on almost everything; `storages` depends on `storage_clients`; `storage_clients` does not import `storages` at module level.
  - But the foundation is not a leaf. There are mutual runtime imports `_utils <-> _request`, `_utils <-> _types`, `_utils <-> errors` and `_utils <-> events`. For example, `_utils/html.py` imports `Request` ([P/src/crawlee/_utils/html.py#L12-L13](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/_utils/html.py#L12-L13)), and `_utils/recoverable_state.py` imports events.
  - `_utils` holds whole domain features (sitemap parsing, HTML form submission), not just helpers.
  - Cycles are dodged with function-local imports (`# noqa: PLC0415`): `_service_locator -> storages` ([P/src/crawlee/_service_locator.py#L123-L132](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/_service_locator.py#L123-L132)) and `RecoverableState -> crawlee.service_locator` ([P/src/crawlee/_utils/recoverable_state.py#L112-L135](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/_utils/recoverable_state.py#L112-L135)).
  - `BasicCrawlingContext` lives in the grab-bag `_types.py` rather than with the crawler, and is re-exported "because of the circular imports" ([P/src/crawlee/crawlers/_basic/__init__.py#L1-L6](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_basic/__init__.py#L1-L6)).
  - `storages`, `sessions`, `browsers` and `_autoscaling` all import the global `crawlee.service_locator` from the root package.
- **Naming:** class-name suffixes are consistent (`*Crawler`, `*CrawlingContext`, `*CrawlerOptions`, `*Client`, `*StorageClient`). Module files mirror the class names (`_beautifulsoup_crawler.py`). The contributing guide fixes the vocabulary, such as `RQ` in code and the full term in docs ([P/CONTRIBUTING.md#L64](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/CONTRIBUTING.md#L64)).
- **File-size discipline:** good overall (median 114 lines), with one dominant exception, `BasicCrawler`:
  - 1,898 lines, 63 methods, 55 attribute assignments.
  - A constructor with 30 parameters (3 of them "private" underscore kwargs for subclasses).
  - `__run_task_function` alone is about 160 lines with 6 `except` branches ([P/.../_basic_crawler.py#L1482-L1643](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_basic/_basic_crawler.py#L1482-L1643)).
  - Ruff ignores every size and complexity rule (`C901`, `PLR0911/0912/0913/0915`; [P/pyproject.toml#L174-L207](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/pyproject.toml#L174-L207)), so nothing caps file or function size.

## 4. Design patterns catalog

| Pattern | Where | Problem it solves | Assessment |
|---|---|---|---|
| Middleware chain (Chain of Responsibility with cleanup) | `ContextPipeline` and `_Middleware`, [P/.../_context_pipeline.py#L24-L146](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_basic/_context_pipeline.py#L24-L146) | Builds the context in steps (navigate, check status, parse, enrich). Each async generator yields exactly once, and teardown runs in reverse order with the consumer's exception. | **Good.** Clean composition (`.compose()` chains at [P/.../_abstract_http_crawler.py#L118-L129](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_abstract_http/_abstract_http_crawler.py#L118-L129)). But each stage needs a new dataclass subclass and a `from_*` copy constructor (10 occurrences), and the error wrapping special-cases `SessionError`. |
| Template Method over a generic class tree | `BasicCrawler -> AbstractHttpCrawler -> BeautifulSoupCrawler/ParselCrawler`, [P/.../_basic_crawler.py#L251-L280](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_basic/_basic_crawler.py#L251-L280) | Shares lifecycle, retries and autoscaling. Subclasses inject a pipeline through the private `_context_pipeline` kwarg. | **Mixed.** It works, but the base is a god class, and the hook is a stringly "private" kwarg checked at runtime ([P/.../_abstract_http_crawler.py#L83-L87](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_abstract_http/_abstract_http_crawler.py#L83-L87)). |
| Strategy (parser) | `AbstractHttpParser`, [P/.../_abstract_http_parser.py#L18-L106](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_abstract_http/_abstract_http_parser.py#L18-L106) | One HTTP crawler serves all HTML libraries, so the BeautifulSoup and Parsel crawlers are about 80 lines each. | **Good.** It was a deliberate fix for duplication, per maintainer issue [#350](https://github.com/apify/crawlee-python/issues/350). |
| Abstract Factory | `StorageClient.create_dataset_client/kvs/rq`, [P/.../_base/_storage_client.py#L18-L72](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/storage_clients/_base/_storage_client.py#L18-L72) | Swaps the whole storage backend (memory, file system, SQL, Redis, Apify) in one place. | **Good.** Clear seam, and backed by a parametrized contract test suite. |
| Facade + Registry (identity map) | `Dataset/KeyValueStore/RequestQueue.open` delegating to `StorageInstanceManager`, [P/.../storages/_dataset.py#L100-L124](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/storages/_dataset.py#L100-L124) | Gives a friendly API and one instance per id, name or alias. | **Mixed.** The cache is process-global, and `open()` is copy-pasted three times (`_key_value_store.py` L112-137, `_request_queue.py` L119-142). |
| Service Locator (global singleton) | [P/src/crawlee/_service_locator.py#L19-L135](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/_service_locator.py#L19-L135) | Provides lazy defaults for `Configuration`, `EventManager` and `StorageClient`, and raises `ServiceConflictError` on late reassignment. | **Bad/mixed.** It is hidden global state: `SessionPool.__init__` writes the global event manager ([P/.../_session_pool.py#L61-L62](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/sessions/_session_pool.py#L61-L62)). There is also a second, per-crawler locator ([P/.../_basic_crawler.py#L380-L395](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_basic/_basic_crawler.py#L380-L395)), so components resolve different instances ([#2242](https://github.com/apify/crawlee-python/issues/2242)). |
| Unit of Work / deferred commit | `RequestHandlerRunResult` records the handler's side effects ([P/src/crawlee/_types.py#L264-L325](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/_types.py#L264-L325)), and the crawler commits them after success ([P/.../_basic_crawler.py#L1419-L1439](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_basic/_basic_crawler.py#L1419-L1439)) | A failed or timed-out handler leaves no partial `push_data`, `add_requests` or key-value writes. It also lets the adaptive crawler run two handlers and keep one result. | **Good.** The strongest idea in the codebase. |
| Observer with typed overloads | `EventManager.on/emit` (one `@overload` per event, data in pydantic models), [P/src/crawlee/events/_event_manager.py#L147-L204](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/events/_event_manager.py#L147-L204) | Decouples periodic persist/system-info signals from storages, statistics and session pools. Listener failures are isolated and logged. | **Mixed.** The typed API is good, but 9 of the 13 `Event` members are never emitted inside crawlee ([P/src/crawlee/events/_types.py#L15-L36](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/events/_types.py#L15-L36)). The browser and session events look like ported names without emitters; `MIGRATING`/`ABORTING` are presumably emitted by the Apify SDK (not verified). |
| Decorator / Composite over request managers | `ThrottlingRequestManager` wraps an inner manager ([P/.../_throttling_request_manager.py#L47-L72](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/request_loaders/_throttling_request_manager.py#L47-L72)). `RequestManagerTandem` pairs a read-only loader with a manager ([P/.../_request_manager_tandem.py#L24-L31](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/request_loaders/_request_manager_tandem.py#L24-L31)). | Adds per-domain backoff, and feeds sitemaps or lists into a queue. | **Mixed.** `BasicCrawler` detects the decorator with `isinstance` in 6 places (L720, 741, 1457, 1651, 1733, 1802), so wrapping it in a tandem silently disables it. Maintainers confirm this in [#2238](https://github.com/apify/crawlee-python/issues/2238) and [#2296](https://github.com/apify/crawlee-python/issues/2296). |
| Router (front controller with label dispatch and middleware) | [P/src/crawlee/router.py#L19-L127](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/router.py#L19-L127) | Maps a request label to a handler, rejects duplicate registration, and runs `use` middlewares first. | **Good.** Small and generic over the context type. |
| Mixin per backend | `SqlClientMixin` ([P/.../_sql/_client_mixin.py#L45-L78](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/storage_clients/_sql/_client_mixin.py#L45-L78)) and `RedisClientMixin` | Shares open, metadata-buffer and purge logic across the three SQL clients (and the three Redis ones). | **Mixed.** It removes duplication within a backend, but `MetadataUpdateParams` is defined twice with different shapes (`_redis/_client_mixin.py` L27-31 uses bools, SQL uses datetimes). |
| Lazy-failure proxy for optional extras | `try_import` plus swapping the module `__class__` to `ImportWrapper`, [P/src/crawlee/_utils/try_import.py#L14-L77](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/_utils/try_import.py#L14-L77) | `from crawlee.crawlers import PlaywrightCrawler` succeeds without the extra, and the first use fails with a `pip install 'crawlee[playwright]'` hint. | **Good UX, magic implementation.** One declaration per symbol group ([P/src/crawlee/crawlers/__init__.py#L9-L107](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/__init__.py#L9-L107)). |
| Memento / recoverable state | `RecoverableState[TStateModel]`, [P/src/crawlee/_utils/recoverable_state.py#L19-L208](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/_utils/recoverable_state.py#L19-L208) | Gives a pydantic state that is persisted on `PERSIST_STATE` events and reloaded on restart. 8 components reuse it (statistics, session pool, file-system RQ, sitemap loader, ...). | **Good DRY.** But it reaches the global locator from inside `_utils`. |
| Composition of sub-crawlers | `AdaptivePlaywrightCrawler` builds a static crawler and a Playwright crawler and runs either or both per request, [P/.../_adaptive_playwright_crawler.py#L375-L448](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_adaptive_playwright/_adaptive_playwright_crawler.py#L375-L448) | Predicts per URL whether JS rendering is needed, using sampled dual runs. | **Mixed.** The idea is strong, but it grabs the sub-crawlers' private `_context_pipeline` and `_browser_pool` (`# noqa: SLF001`, L228-233). Inheritance-based crawlers weren't built to be composed. |
| Plugin + pool | `BrowserPool` manages `BrowserPlugin -> BrowserController` ([P/src/crawlee/browsers/_browser_plugin.py#L19-L91](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/browsers/_browser_plugin.py#L19-L91)) with pre/post launch and page hooks | Rotates browsers and caps pages per browser. Ported from JS `browser-pool`. | **Good seam.** But the Playwright and Stagehand controllers share 51 code lines in identical runs (similarity ratio 0.44). |
| Async-context-manager lifecycle with an `active` flag | Hand-rolled in 10 files (HttpClient, SessionPool, Statistics, BrowserPool, Snapshotter, ...), each with its own "already active" `RuntimeError` (16 copies) | Lets the crawler enter and exit collaborators. | **Bad.** `_run_crawler` enters whatever is not yet active ([P/.../_basic_crawler.py#L821-L853](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_basic/_basic_crawler.py#L821-L853)), so ownership is implicit and shared collaborators break ([#2301](https://github.com/apify/crawlee-python/issues/2301); typestate idea [#740](https://github.com/apify/crawlee-python/issues/740)). |

## 5. Separation of concerns and DRY

**What keeps it DRY:**
- The pipeline steps and the parser strategy (section 4).
- The generic `BasicCrawler._enqueue_links_filter_iterator` and `_create_enqueue_links_function`, shared by all crawlers ([P/.../_basic_crawler.py#L1051-L1155](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_basic/_basic_crawler.py#L1051-L1155)).
- `RecoverableState` and `try_import`.
- In tests, the storages contract suite parametrized over `memory`, `file_system`, `sql` and `redis` ([P/tests/unit/storages/conftest.py#L33](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/tests/unit/storages/conftest.py#L33)), and the HTTP client fixture parametrized over httpx, impit and curl ([P/tests/unit/conftest.py#L279-L299](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/tests/unit/conftest.py#L279-L299)). That parametrization was itself a debt ticket ([#1292](https://github.com/apify/crawlee-python/issues/1292)).
- The agent rules file is kept DRY with symlinks: `AGENTS.md`, `CLAUDE.md` and `GEMINI.md` all point to `.rules.md`.

**Where DRY fails** (measured with a line-similarity script: identical runs of at least 5 code lines, ignoring blanks, imports and comments):
- **Sibling crawlers without a shared middle layer.**
  - `AbstractHttpCrawler` and `PlaywrightCrawler` share 72 such lines.
  - `_manage_shared_navigation_timeout` is duplicated verbatim (HTTP L131-141 vs Playwright L232-242), and so is `_execute_post_navigation_hooks`.
  - `_handle_status_code_response` is nearly identical (HTTP [L295-322](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_abstract_http/_abstract_http_crawler.py#L295-L322) vs Playwright [L500-525](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_playwright/_playwright_crawler.py#L500-L525)).
  - The roughly 80-line `extract_links` closure differs only in where the links and the base URL come from (HTTP [L185-272](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_abstract_http/_abstract_http_crawler.py#L185-L272) vs Playwright [L414-498](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_playwright/_playwright_crawler.py#L414-L498)).
  - [#350](https://github.com/apify/crawlee-python/issues/350) records the original choice: avoid JS-style inheritance chains and pay with "a bit of code duplication".
- **Storage backends.**
  - The block that warns about `get_data`/`iterate` arguments the backend doesn't support is copy-pasted 7 times across 4 backends (e.g. [P/.../_memory/_dataset_client.py#L150-L166](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/storage_clients/_memory/_dataset_client.py#L150-L166), [P/.../_sql/_dataset_client.py#L258-L273](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/storage_clients/_sql/_dataset_client.py#L258-L273), redis L165 and L254, fs L256 and L353).
  - The root cause is a leaky interface: the abstract `DatasetClient.get_data` mirrors Apify API parameters (`clean`, `fields`, `omit`, `unwind`, `flatten`, `view`) that no local backend implements ([P/.../_base/_dataset_client.py#L53-L66](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/storage_clients/_base/_dataset_client.py#L53-L66)).
  - The memory and file-system dataset clients share 93 lines in identical runs.
  - The file-system clients repeat the "scan dirs, load `__metadata__.json`, validate" logic for each storage type (for example, dataset vs RQ client: 50 shared lines).
- **HTTP clients.** impit, httpx and curl each reimplement `crawl`/`send_request` error mapping, redirects and cookies (impit vs httpx: 60 shared lines). Maintainers plan a `BaseHttpClient` with a per-hop primitive ([#2307](https://github.com/apify/crawlee-python/issues/2307)).
- **Options declared three times.**
  - Every `BasicCrawler` option appears as a `TypedDict` field with a docstring ([P/.../_basic_crawler.py#L117-L248](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_basic/_basic_crawler.py#L117-L248)), again as a constructor parameter (L280-314), and again in the `Args:` docstring (L315-372).
  - The `TypedDict` exists so subclasses can forward `**kwargs: Unpack[...]`. There are 41 `Unpack[` uses.
- **Per-model config.** 28 models repeat the same `ConfigDict(validate_by_name=True, validate_by_alias=True, ...)`, by explicit policy ([P/.rules.md#L11-L13](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/.rules.md#L11-L13): ruff's type-checking rules only recognize a literal `BaseModel` base).
- **Error-handler logic** is duplicated inside `__run_task_function`: the `SessionError` branch re-implements error-handler invocation and failure marking (L1597-1611) next to `_handle_error_handler_replacement` and `_handle_failed_request`.

**Separation of concerns:**
- `BasicCrawler` owns all of the following: request lifecycle, retries, session rotation, robots.txt fetching and caching, 429 throttling, statistics, root-logger configuration, SIGINT handling, purge-on-rerun policy, and dataset export to CSV/JSON.
- `Request` mixes the stored record with crawler state kept under `user_data['__crawlee']` ([#2302](https://github.com/apify/crawlee-python/issues/2302)).
- `Configuration` accepts `APIFY_*` environment aliases in the open-source core ([P/src/crawlee/configuration.py#L38-L75](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/configuration.py#L38-L75)).
- OpenTelemetry support monkeypatches internals, including the name-mangled `_BasicCrawler__run_task_function` and the private `_Middleware` ([P/src/crawlee/otel/crawler_instrumentor.py#L108-L117](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/otel/crawler_instrumentor.py#L108-L117)).

**Comparison with JS Crawlee** (spot checks at [`apify/crawlee@0289924`](https://github.com/apify/crawlee/tree/02899241d570624a0ce0b77dbcb44b9d58a98ade), 2026-10-15):
- **Packaging.** JS is a monorepo of 19 npm packages (`basic-crawler`, `browser-crawler`, `http-crawler`, `cheerio-crawler`, `playwright-crawler`, `browser-pool`, `http-client`, `impit-client`, `core`, `types`, `utils`, ...). Python is one distribution where extras plus `try_import` stand in for package boundaries.
- **Base crawler size.** The JS `basic-crawler.ts` is 3,180 lines, so the god base class is inherited from the original design, not introduced by the port.
- **Hierarchy.** JS has shared middle layers (`browser-crawler.ts` 915 lines, `http-crawler.ts` 1,010). Python instead has a parser strategy plus duplicated siblings.
- **Context pipeline.** Python has had `ContextPipeline` since its first `BasicCrawler` (PR #56, April 2024). JS master now has one too ([`context-pipeline.ts`](https://github.com/apify/crawlee/blob/02899241d570624a0ce0b77dbcb44b9d58a98ade/packages/basic-crawler/src/internals/crawlers/context-pipeline.ts)), with a different shape: a middleware returns an extension object that is merged into the context (`TContext & TExtension`), and teardown is registered via `onCleanup`. My inference: that shape avoids Python's one-dataclass-subclass-per-stage, which Python needs because it lacks intersection types.
- **Error hierarchy.** JS has a retry-semantics hierarchy (`NonRetryableError`, `CriticalError`, `RetryRequestError`) that Python lacks ([#1739](https://github.com/apify/crawlee-python/issues/1739)).
- **How the ports stay in sync: by hand, with no shared spec.**
  - 14 core files carry `# Inspiration:` headers pinned to old JS tags (v3.7.3 to v3.10.1), e.g. [P/.../_basic_crawler.py#L1](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_basic/_basic_crawler.py#L1).
  - Recent issues port JS v4 changes one by one, each with a "JS references" section (#2296, #2301, #2302, #2307).
  - The wire format (camelCase JSON shared with JS and the Apify platform) is hand-written: 94 manual `alias='camelCase'` entries on `master`. The `v2` branch switches to `alias_generator=to_camel` ([PR #2319](https://github.com/apify/crawlee-python/pull/2319)).

## 6. Contracts and extension points

- **Extension points** (official guide [P/docs/guides/extending_crawlee.mdx#L13-L112](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/docs/guides/extending_crawlee.mdx#L13-L112)):
  - Subclass `BasicCrawler`, `AbstractHttpCrawler` (with an `AbstractHttpParser`) or `PlaywrightCrawler`.
  - Implement `HttpClient` (`crawl`, `send_request`, `stream`, `cleanup`; [P/src/crawlee/http_clients/_base.py#L74-L225](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/http_clients/_base.py#L74-L225)).
  - Implement `StorageClient` with the three sub-clients, or `BrowserPlugin`/`BrowserController`.
  - In user code: router middleware, pre/post navigation hooks, `error_handler`, `failed_request_handler`, `on_skipped_request`, and `register_deferred_cleanup` on the context.
  - Plug-in is by ABC subclassing. There is no entry-point registry.
- **Configuration:**
  - `Configuration(BaseSettings)` reads `CRAWLEE_*` environment variables (and `APIFY_*` aliases).
  - Services are registered globally (`service_locator.set_*`, which raises `ServiceConflictError` after first use), per crawler (constructor kwargs), or per storage (`open(storage_client=...)`) ([P/docs/guides/service_locator.mdx#L48-L130](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/docs/guides/service_locator.mdx#L48-L130)).
  - Several resolution paths for the same services are the root of [#1653](https://github.com/apify/crawlee-python/issues/1653) and [#2242](https://github.com/apify/crawlee-python/issues/2242).
- **Errors** ([P/src/crawlee/errors.py#L29-L140](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/errors.py#L29-L140)):
  - 14 exception classes with no common `CrawleeError` base: 11 subclass `Exception` directly, and 3 subclass another Crawlee error (`UserHandlerTimeoutError`, `ProxyError`, `HttpClientStatusCodeError`).
  - Wrapper errors carry the crawling context (`RequestHandlerError[TCrawlingContext]`, `ContextPipelineInitializationError`).
  - Retry semantics are decided by `isinstance` branches in `_should_retry_request` and `__run_task_function`, not declared on the error types (`SessionError` means rotate the session, `HttpClientStatusCodeError` means never retry).
  - `UserHandlerTimeoutError` is exported from neither `__all__` nor the docs.
- **Logging and observability:**
  - Stdlib `logging`, with `getLogger(__name__)` in 58 places.
  - The crawler configures the root logger only if the host has not already ([P/.../_basic_crawler.py#L458-L464](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/crawlers/_basic/_basic_crawler.py#L458-L464)).
  - Periodic statistics come as a table or inline, from `Statistics` (generic over a pydantic state model), `ErrorTracker` and `ErrorSnapshotter`.
  - OpenTelemetry is available through an optional `CrawlerInstrumentor`.
  - Log text, levels and logger names are treated as a public contract.
- **Deprecation:**
  - `warnings.warn(..., DeprecationWarning)` naming the removal version, e.g. [P/src/crawlee/fingerprint_suite/_header_generator.py#L57-L62](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/fingerprint_suite/_header_generator.py#L57-L62) and [P/src/crawlee/statistics/_models.py#L123-L129](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/statistics/_models.py#L123-L129).
  - Experimental features (`SqlStorageClient`, `RedisStorageClient`, `PydanticAiCrawler`) raise a one-time `UserWarning`.
  - Breaking changes accumulate on a `v2` branch (10 commits ahead of `master`, including dropping Python 3.10 and making `RequestQueueClient.is_finished` abstract). Users get upgrade guides (`docs/upgrading/upgrading_to_v1.md`, 347 lines) and Conventional Commits with `!`.

## 7. Quality machinery

- **Ruff** ([P/pyproject.toml#L159-L268](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/pyproject.toml#L159-L268)):
  - `select = ["ALL"]` with 27 ignores, each with a comment. The notable ignores are all complexity and size rules (`C901`, `PLR0911/0912/0913/0915`), `BLE001` (blind except), `ANN401` (`Any`), `EM`, `TRY003` and `G004`.
  - Tests may access private members (`SLF001`) and skip docstrings.
  - Line length is 120 (90 for docs examples, so they render without scrolling), with single quotes.
  - Ruff also lints and formats Python code blocks inside `.md`/`.mdx`.
  - pydocstyle (Google convention) applies to public source.
- **Types:** `ty check` over `src`, `tests`, `scripts`, `docs` and `website` with default rules. Imports are unresolved-tolerant only for docs.
- **Tests:**
  - pytest with `asyncio_mode=auto` and xdist work-stealing.
  - `run_alone` tests run first, serially.
  - Global timeout 1,800 s, plus `faulthandler` at 600 s.
  - A local uvicorn test server ([P/tests/unit/server.py](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/tests/unit/server.py)), `proxy-py` for proxies, and `fakeredis`.
  - Flakiness policy in [P/tests/unit/README.md](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/tests/unit/README.md), applied with `flaky`/`skip` in 12 places.
  - Global singletons force an autouse fixture that resets private state, including name-mangled class counters ([P/tests/unit/conftest.py#L61-L84](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/tests/unit/conftest.py#L61-L84)).
  - I found no test that executes the `docs/` code examples. They are only linted and type-checked.
- **CI** ([P/.github/workflows/_checks.yaml](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/.github/workflows/_checks.yaml)):
  - Linters: actionlint, typos, markdownlint, oxlint/oxfmt for the website, and a check that rejects unoptimized images.
  - Lint and type checks run on 6 Python versions.
  - Unit tests run on 6 Python versions × 3 operating systems.
  - A package build plus an import smoke test, and a pip<26 resolution check of `crawlee[all]`.
  - A docs build, and a PR title check (Conventional Commits).
  - A weekday cron runs end-to-end template tests over 10 crawler types × 3 HTTP clients × 3 package managers on the Apify platform ([P/.github/workflows/on_schedule_tests.yaml#L39-L47](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/.github/workflows/on_schedule_tests.yaml#L39-L47)).
  - Third-party actions are pinned by SHA, but the shared Apify reusable workflows are referenced at `@main`.
  - Codecov is informational only (it never fails the build; [P/codecov.yaml](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/codecov.yaml)).
  - An AI review and auto-approval workflow exists, with deny-globs ([P/.github/workflows/factory_review.yaml](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/.github/workflows/factory_review.yaml)).
  - Pre-commit runs `poe lint` and `poe type-check`.
  - `uv` installs only packages published at least 24 hours ago, as a supply-chain guard ([P/pyproject.toml#L320-L330](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/pyproject.toml#L320-L330)).
- **Contribution rules** ([P/CONTRIBUTING.md#L50-L127](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/CONTRIBUTING.md#L50-L127)):
  - Readability over density.
  - Comments describe the present code only.
  - ASCII-only source.
  - Flat `test_*` functions with one-line docstrings and `pytest.param(id=...)`.
  - Tests must restore any process-wide state they change.
  - Conventional Commits.
  - Log changes are breaking.
- **Design docs:** there are no ADRs or design docs in the repo. Architecture is explained in the user guide ([P/docs/guides/architecture_overview.mdx](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/docs/guides/architecture_overview.mdx)) and the agent rules file. Design debate lives in GitHub issues, e.g. #350 with a linked gist.

## 8. Pain points

What the maintainers say in issues (all open unless noted; several were filed 2026-10-13 as a v2 batch, drafted with Claude Code per their footers):
- **Lifecycle ownership.** A crawler enters and exits collaborators based on whether they are `active`, not on who created them. Shared `Statistics`, `SessionPool` or `BrowserPool` instances therefore break, and per-run flags live on the crawler. The plan is owned-vs-injected collaborators plus a per-run `CrawlerRun` object, ported from JS v4 ([#2301](https://github.com/apify/crawlee-python/issues/2301)). A 2024 maintainer note proposed typestate-style context managers ([#740](https://github.com/apify/crawlee-python/issues/740)).
- **Global vs per-crawler services.** A crawler-scoped `EventManager` runs a duplicate sampling loop while `Snapshotter` and `RecoverableState` listen on the global one. Standalone storages never get periodic persistence ([#2242](https://github.com/apify/crawlee-python/issues/2242)). A user reported a second crawler silently reusing the first crawler's queue ([#1653](https://github.com/apify/crawlee-python/issues/1653), closed). An earlier proposal to make the test reset easier was a `lazy_object_proxy` for `service_locator` ([#884](https://github.com/apify/crawlee-python/issues/884), closed).
- **Capability probing via `isinstance`.** This is the root cause of `ThrottlingRequestManager` being ignored behind a tandem ([#2238](https://github.com/apify/crawlee-python/issues/2238)). The fix replaces `is_empty`/`is_finished` with `check_readiness()` and forwards pacing signals through every wrapper ([#2296](https://github.com/apify/crawlee-python/issues/2296)).
- **Duplicated HTTP-client logic** for redirects and cookies ([#2307](https://github.com/apify/crawlee-python/issues/2307)).
- **`Request` conflates storage and crawler views** ([#2302](https://github.com/apify/crawlee-python/issues/2302)).
- **Thin error hierarchy compared with JS** ([#1739](https://github.com/apify/crawlee-python/issues/1739)).
- **Crawler inheritance** reconsidered in 2024 ([#350](https://github.com/apify/crawlee-python/issues/350), closed with the parser-strategy design).
- **Older debt tickets:** refactor `__run_task_function` ([#115](https://github.com/apify/crawlee-python/issues/115), closed 2024; the method is still about 160 lines) and status-code argument handling ([#998](https://github.com/apify/crawlee-python/issues/998)).

My own inferences, not stated by maintainers:
- The `_utils` package is a junk drawer with back-edges into domain modules.
- Private modules double as the Apify SDK's interface, which freezes internals.
- The Apify-shaped dataset interface pushes unsupported-argument boilerplate into every backend.
- The file-system clients offload `open`/`close` to threads but call `json.load` and `iterdir()` on the event loop ([P/.../_file_system/_dataset_client.py#L124-L153](https://github.com/apify/crawlee-python/blob/508f544f7f0260b4f26b806d50e84592538f8db6/src/crawlee/storage_clients/_file_system/_dataset_client.py#L124-L153)).

## 9. Scorecard

| Area | Score (1-5) | Justification |
|---|---|---|
| Layering and dependency direction | 3 | The top-down shape holds (crawlers, then storages, then storage clients). But `_utils` has 4 mutual runtime cycles, the global `service_locator` is imported from the root package everywhere, and cycles are dodged with local imports. |
| Module and file size discipline | 3 | Median file is 114 lines. But the 1,898-line `BasicCrawler` has a 160-line method, and every complexity and size lint is disabled. |
| DRY (including across language ports) | 2 | Measured sibling copy-paste (HTTP vs Playwright, 4 storage backends, 3 HTTP clients). Options are declared 3 times. The JS port is synced by hand, with no shared schema. |
| Separation of concerns | 2 | `BasicCrawler` owns about 10 concerns. `Request` mixes storage and crawler state. Apify specifics leak into the core config and interfaces. |
| Extension model | 4 | Clear ABC seams (HttpClient, StorageClient, Parser, BrowserPlugin), router middleware, hooks, and a documented extension guide. Subclass hooks go through underscore kwargs. |
| Contracts and types | 3 | Strong generics, Protocols and pydantic wire models. But type checking is non-strict (`ty` defaults, 183 `Any`, 34 casts), and private modules are de-facto API. |
| Error model | 2 | Flat hierarchy with no base. Retry policy lives in `isinstance` ladders, not in types. The gap is acknowledged in #1739. |
| Test architecture | 4 | 1,297 tests, parametrized backend contract suites, a local server, a 6 × 3 OS/Python matrix and an e2e template matrix. Global-state reset fixtures and informational-only coverage cost a point. |
| Tooling and CI enforcement | 4 | Ruff `ALL`, typos, actionlint, markdownlint, package smoke tests, SHA-pinned actions, commit-title gate. But complexity lints are off, coverage never fails, and the reusable workflows float at `@main`. |
| In-code documentation | 4 | Docstrings are enforced and there are attribute docstrings. `docs_group` drives the API reference, and an architecture guide exists. Option docs are duplicated, and the `Inspiration` links point to stale JS tags. |

## 10. Lessons for Goodfetch

- **Adopt the deferred-commit unit of work.** Have handlers record their effects (data items, enqueues, key-value writes) into a per-request buffer, and have the crawler commit it only after success (`RequestHandlerRunResult`, section 4). It makes retries clean and enables "run twice, keep one" strategies like the adaptive crawler. In Foxhound, model it as the staged Records of ADR 0103, returned or accumulated per request, so no partial writes reach storage. That matters when Collie (browser) and HTTP fetches race for the same request.
- **Adopt typed per-stage Hooks (ADR 0104), but without one subclass per stage.** Python's yield-once middlewares with guaranteed reverse cleanup are sound. The cost is a dataclass subclass and a copy constructor at every stage, plus special-cased errors. JS moved to "return an extension, register a cleanup". In Rust, prefer a context struct with typed extension fields (or typestate generics) and a `Drop`/explicit cleanup stack. Spawn any tasks through the owner's task tracker, as Goodfetch already requires.
- **Don't build a god base crawler, and declare options once.** `BasicCrawler` has 30 constructor parameters, 63 methods, and options written three times (section 5). Goodfetch already generates its SDKs from the protocol crate. Make one Rust options struct the single source for SDK parameters, CLI flags, MCP schemas and docs. Enable clippy `too_many_lines`, `too_many_arguments` and `cognitive_complexity` with thresholds instead of turning them off, as Crawlee's ruff config does.
- **Express capabilities in the interface; never discover them by concrete type.** Six `isinstance(…, ThrottlingRequestManager)` checks made the feature silently vanish behind a wrapper (#2238, #2296). Goodfetch treats capabilities as runtime settings. Expose them as trait methods or signals that every wrapper forwards (e.g. `record_pacing_signal`, `check_readiness`), never as `Any::downcast_ref`, and return "nobody consumed this signal" explicitly.
- **Avoid global service locators; make ownership explicit in types.** Crawlee's global locator plus per-crawler locators produced duplicate event managers, shared-queue surprises and test fixtures that reset private globals (#1653, #2242, #2301). In Rust, separate owned collaborators (`T`, started and stopped by the crawler) from borrowed ones (`Arc<T>`, whose lifecycle the caller owns). Put per-run state in a `CrawlerRun` value. Keep no process-wide mutable singletons.
- **Design the error kinds up front, as data.** Crawlee's retry, rotate-session, non-retryable and fatal semantics live in `isinstance` ladders, and Python never ported JS's hierarchy (#1739). Give Goodfetch's protocol errors an explicit retry and lifecycle classification, plus stable codes generated into every SDK. That prevents per-language drift.
- **Keep private things private between sibling products.** The Apify SDK imports crawlee's `_`-modules and subclasses its file-system clients, so the maintainers must treat internals as API (`.rules.md` L61-70). Foxhound contains Collie, so Foxhound should consume Collie only through Collie's public crate API, with `pub(crate)` everywhere else. That is the mirror of ADR 0087's rule.
- **Make observability names part of the contract, and don't derive them from module paths.** Crawlee treats logger names and messages as breaking changes, yet those names are private module paths, and OpenTelemetry works by monkeypatching mangled private methods. Give Goodfetch stable `tracing` targets, span names and event fields defined in one place, with first-class spans around the pipeline stages, so internal refactors stay free.
- **Keep core storage and transport traits minimal.** A vendor-shaped `get_data(clean, fields, omit, unwind, flatten, view)` forced 7 copies of unsupported-argument warnings. Put vendor extras in extension traits or capability-gated methods, and implement cross-cutting behaviour (redirects, cookies, metadata loading) once in a shared base layer, which is the fix Crawlee is moving to (#2307).
- **Adopt the conformance-suite testing style and the strict lint stance, and add what Crawlee lacks.** Parametrize one trait-level suite over every backend and transport, as Crawlee does for its 4 storage backends and 3 HTTP clients. Lint and type-check the docs examples. Go further: execute the doc snippets in CI, make coverage and complexity gates blocking, and pin the reusable workflows.
