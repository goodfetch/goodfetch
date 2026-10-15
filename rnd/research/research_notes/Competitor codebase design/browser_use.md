# Browser Use (browser-use/browser-use): codebase design study (as of 2026-10-16)

Evidence was read from a local shallow clone at commit `a71d5478e4187cbe5295aab6403dfe7e4a6861a4` (committed 2026-10-15). Code links are permalinks at that commit. Issue, PR and release links were read through `gh` on 2026-10-16.

How to read the tags:
- An untagged finding was verified in the code, the repository's config, or the cited page.
- `[inferred: basis]` marks my own deduction.
- "Maintainers say" marks a statement by an OWNER, MEMBER or CONTRIBUTOR on the cited page. Reporter claims are labelled as such.
- Counts come from `tokei`, `wc -l`, `grep`, and an AST import scan of `browser_use/` (it parses the code and never runs it).

Permalink base: `https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/`

## 1. Snapshot

- **Repo:** browser-use/browser-use, package `browser-use` 0.13.11, Python `>=3.11`, MIT ([pyproject.toml#L1-L12](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/pyproject.toml#L1-L12)).
- **Languages (tokei, whole repo):** Python has 411 files and 85,627 code lines. Markdown has 60 files and 7,815 lines. There are a few Dockerfiles, shell scripts and TOML/YAML files, plus 3 HTML fixtures.
  - The repo contains no Rust.
  - The new Rust core ships separately as the platform wheel `browser-use-core`, behind the `core` extra ([pyproject.toml#L61-L69](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/pyproject.toml#L61-L69)).
- **Split:**

  | Tree | Files | Lines | Code lines |
  |---|---|---|---|
  | `browser_use/` | 175 | 67,953 | 52,690 |
  | `tests/` | 124 | 35,494 | 26,531 |
  | `examples/` | 111 | 8,800 | 6,343 |

- **Structure:** 17 subpackages and 9 top-level modules in 49 directories. They hold 320 classes and 2,135 functions, of which 664 are `async`.
- **Code lines per subpackage (tokei):**

  | Subpackage | Files | Code lines | Note |
  |---|---|---|---|
  | `browser` | 29 | 14,789 | |
  | `llm` | 54 | 7,838 | |
  | `beta` | 2 | 6,217 | Almost all in one file |
  | `agent` | 12 | 5,779 | |
  | `dom` | 12 | 4,510 | |
  | `tools` | 8 | 2,665 | |
  | `actor` | 8 | 1,851 | |
  | `mcp` | 6 | 1,761 | |
  | `integrations` | 8 | 1,520 | |
  | `tokens` | 7 | 835 | |
  | `filesystem` | 2 | 833 | |
  | `sandbox` | 3 | 642 | |
  | `skills` | 6 | 580 | |
  | `sync` | 3 | 341 | |
  | `telemetry` | 3 | 212 | |
  | `screenshots` | 2 | 34 | |
  | `controller` | 1 | 2 | Alias shim |

  The top-level modules add 3,065 lines. The largest are `utils.py` (902), `config.py` (529), `cli.py` (464) and `init_cmd.py` (448).
- **File sizes:**
  - The median module is 184 lines.
  - 35 modules exceed 500 lines, 14 exceed 1,000 and 5 exceed 2,000.
  - The 6 largest files hold 22,716 lines, 33% of the package.
- **10 largest source files (lines):**

  | Rank | File | Lines |
  |---|---|---|
  | 1 | `beta/service.py` | 6,810 |
  | 2 | `agent/service.py` | 4,163 |
  | 3 | `browser/session.py` | 4,155 |
  | 4 | `browser/watchdogs/default_action_watchdog.py` | 3,758 |
  | 5 | `tools/service.py` | 2,327 |
  | 6 | `browser/watchdogs/downloads_watchdog.py` | 1,503 |
  | 7 | `dom/serializer/serializer.py` | 1,406 |
  | 8 | `browser/profile.py` | 1,300 |
  | 9 | `mcp/server.py` | 1,294 |
  | 10 | `dom/service.py` | 1,245 |

  The largest test file is `tests/ci/test_beta_agent.py`, at 7,920 lines.

## 2. Paradigm

- **Runtime and concurrency**
  - The library is async-first on a single asyncio loop. The repo's agent file says "Use async python" ([CLAUDE.md#L59-L61](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/CLAUDE.md#L59-L61)).
  - Sync use goes through `Agent.run_sync`, which calls `asyncio.run` ([agent/service.py#L4061-L4070](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L4061-L4070)).
  - Background work is fire-and-forget: 37 call sites use `create_task_with_error_handling`, and nothing owns those tasks ([utils.py#L832-L887](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/utils.py#L832-L887)).
  - Watchdog tasks are cancelled by a `__del__` hook. It reflects over private attributes whose names end in `_task` or `_tasks` ([watchdog_base.py#L293-L321](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/watchdog_base.py#L293-L321)).
- **Style:** object-oriented, with pydantic models used as services.
  - `BrowserSession` and `BaseWatchdog` are `BaseModel`s. They keep runtime state in `PrivateAttr`, so configuration, mutable state and behaviour share one class.
  - LLM providers are `@dataclass`es that satisfy a runtime-checkable `Protocol` ([llm/base.py#L32-L74](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/llm/base.py#L32-L74)).
  - Inheritance is shallow, with one exception. `BrowserProfile` inherits from four classes that mirror Playwright's argument groups ([profile.py#L585](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/profile.py#L585)).
- **Event-driven browser layer**
  - The browser layer is built on the `bubus` event bus and 45 `BaseEvent` subclasses.
  - The agent loop is imperative. Each step runs prepare context → get next action → execute actions → post-process, inside one `try/except/finally` ([agent/service.py#L1035-L1085](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L1035-L1085)).
- **Public API style**
  - Constructors take many keyword arguments.
    - `Agent.__init__` has 62 named parameters plus `**kwargs` ([agent/service.py#L136-L214](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L136-L214)).
    - `BrowserSession` has three `@overload`ed constructors: cloud, local and an implementation ([session.py#L162-L300](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/session.py#L162-L300)).
  - Pairs of aliases abound: `browser`/`browser_session`, `tools`/`controller`, `skills`/`skill_ids`, `Browser = BrowserSession` and `Controller = Tools`.
  - Exports are lazy, through PEP 562 `__getattr__` ([__init__.py#L76-L137](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/__init__.py#L76-L137)).
  - Custom tools are declared with a decorator.
- **Typing strictness:** pyright runs in `basic` mode.
  - It excludes the 6,810-line `beta/service.py` and its test ([pyproject.toml#L145-L174](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/pyproject.toml#L145-L174)).
  - The package has 112 `# type: ignore`, 722 `Any` tokens, 293 `getattr(` and 151 `hasattr(` calls.
  - Key models do use `extra='forbid'`.

## 3. Organization

- **Top level**
  - Code lives in `browser_use/`, `tests/` (`ci/`, `agent_tasks/`, `mind2web_data/`, `scripts/`) and `examples/`, which is grouped by theme.
  - Tooling sits in `docker/`, `bin/` (`lint.sh`, `test.sh`, `setup.sh`), `scripts/`, `skills/` and `static/`.
  - Four agent and doc files sit at the root: `AGENTS.md` (1,017 lines), `CLAUDE.md` (163), `CLOUD.md` (2,704) and `BETA_AGENT_INTEGRATION_FEATURES.md` (348).
  - There is no `docs/` directory, even though `CLAUDE.md` tells agents to update one ([CLAUDE.md#L119](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/CLAUDE.md#L119)).
- **Package boundaries**

  | Subpackage | Contents |
  |---|---|
  | `agent/` | Orchestrator, `message_manager/`, `prompts.py`, 8 Markdown system prompts |
  | `browser/` | `session.py`, `profile.py`, `events.py`, `session_manager.py`, `watchdogs/` (14 files), `cloud/` |
  | `dom/` | `service.py`, `serializer/` |
  | `tools/` | `service.py` with the built-in actions, `registry/` |
  | `llm/` | One folder per provider, each with `chat.py` and `serializer.py` |
  | `actor/` | Playwright-like `Page`/`Element`/`Mouse` over CDP |
  | `mcp/` | Server, client and controller |
  | `beta/` | Rust bridge |

  Smaller packages handle files, tokens and cost, telemetry, cloud sync, sandbox, skills and integrations.
- **The `service.py`/`views.py` convention**
  - It comes from a Netflix Dispatch-inspired guide: "Always a single file, except if it becomes too long - more than ~500 lines, split it" ([browser_use/README.md#L24-L26](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/README.md#L24-L26)).
  - It is followed in `agent`, `agent/message_manager`, `dom`, `tools`, `tools/registry`, `skills`, `telemetry` and `tokens`.
  - It is not followed in `browser`, `llm`, `mcp`, `actor` or `filesystem`.
  - The 500-line cap is broken by 35 modules, and 14 of them are over 1,000 lines.
- **Dependency direction**
  - `llm` is a clean leaf: at module level it imports only `observability`.
  - `agent` depends on `browser`, `dom`, `llm`, `tools`, `filesystem`, `tokens` and `telemetry`. `browser` depends on `dom`, `actor`, `utils` and `observability`.
  - **Cycles:**
    - `agent ↔ tools` is a cycle at module level. `tools/service.py` imports `ActionModel` and `ActionResult` from `agent.views` ([tools/service.py#L16](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/service.py#L16)), so the core result type lives in the consumer's package.
    - `browser ↔ dom` and `browser ↔ actor` are cycles hidden by `TYPE_CHECKING` or function-level imports ([dom/service.py#L30](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/dom/service.py#L30), [session.py#L62](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/session.py#L62)).
    - The stable `agent` package lazily re-exports `BetaAgentError` from `beta` ([agent/__init__.py#L9-L13](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/__init__.py#L9-L13)).
- **Encapsulation leaks**
  - Code outside `browser/` reads `BrowserSession` private attributes 9 times, for example `_cached_browser_state_summary`, `_cdp_client_root`, `_reconnect_event` and `_dom_watchdog`.
  - The agent also reaches through two registry layers: `self.tools.registry.registry.actions` ([agent/service.py#L2815](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L2815)).
- **Public vs internal**
  - The split is by convention only. The package has one underscore module (`browser/_cdp_timeout.py`), and the root `__all__` lists about 30 names, with `Controller` listed twice.
  - Examples import deep modules: `browser_use.agent.service` 8 times, `agent.views` 6 times and `browser.events` twice. In effect the whole tree is public API `[inferred: examples are the user contract]`.
- **Naming conventions**
  - Classes are named `Chat<Provider>`, `<Concern>Watchdog` and `<Thing>Event`, and handlers are named `on_<EventName>`.
  - Event names must not be substrings of each other, which is checked at import time ([events.py#L641-L668](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/events.py#L641-L668)).
  - Code uses tabs, single quotes and a 130-column limit.
- **Tests and fixtures**
  - `tests/ci/` holds 119 `test_*.py` files, grouped into `browser/`, `models/`, `interactions/`, `infrastructure/`, `security/` and `integrations/`. A shared `conftest.py` provides a mock LLM.
  - Some unit tests live inside the package, in `browser_use/llm/tests` and `browser_use/tokens/tests`. The wheel excludes them ([pyproject.toml#L177-L189](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/pyproject.toml#L177-L189)).
  - The same include list names `browser_use/cli_templates/*.py` and `browser_use/dom/**/*.js`. Neither exists, so the build config is stale.

## 4. Design patterns catalog

| Pattern | Where | Problem it solves | Assessment |
|---|---|---|---|
| Typed event bus (observer/mediator) with result-typed events and per-event timeouts | [events.py#L110-L122](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/events.py#L110-L122), dispatch from [tools/service.py#L502-L512](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/service.py#L502-L512) | Decouples tools and agent from browser internals, and lets the page raise events on its own (downloads, crashes, dialogs) | **Mixed.** The idea is sound. But the third-party bus needed a subclass patch for a teardown bug ([session.py#L106-L131](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/session.py#L106-L131)). It also has a cross-loop bug ([#5509](https://github.com/browser-use/browser-use/issues/5509)). Events carry DOM graph nodes, so a validator strips cycles ([events.py#L51-L81](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/events.py#L51-L81)). Tools bypass the bus 6 times with direct `cdp_client.send` calls. |
| Handler registration by naming convention, plus a declared `LISTENS_TO`/`EMITS` contract | [watchdog_base.py#L243-L291](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/watchdog_base.py#L243-L291) | Little ceremony to subscribe. Duplicate registration raises. | **Mixed.** Registration relies on reflection over `dir(events)`. The contract is checked only when `LISTENS_TO` is non-empty, and the largest watchdog declares none. |
| Wrapper around every handler: circuit breaker, wait for reconnect, CDP session repair, causal debug log | [watchdog_base.py#L56-L222](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/watchdog_base.py#L56-L222) | One home for connection-loss policy, so handlers don't hang on a dead WebSocket | **Good** centralisation. However, it detects error types by string (`'ConnectionClosedError' in str(type(e))`, [L194](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/watchdog_base.py#L194)). |
| One component per concern ("watchdogs") | [session.py#L1690-L1839](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/session.py#L1690-L1839) | Splits downloads, popups, security, DOM, screenshots, storage, HAR, captcha and permissions out of the session | **Mixed.** It is a real separation-of-concerns win. But the wiring is hard-coded, carries commented-out legacy calls, and runs some watchdogs only under conditions. `DefaultActionWatchdog` is the action executor for 15 events and is "watchdog" in name only. |
| Registry plus decorator for agent actions | [registry/service.py#L291-L329](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/registry/service.py#L291-L329) | Users and built-ins add LLM-callable actions with descriptions, domain filters and `terminates_sequence` | **Good.** It is simple and declarative, and it rejects `**kwargs` in action signatures ([L92-L99](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/registry/service.py#L92-L99)). |
| Dependency injection by parameter name | [registry/service.py#L57-L73](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/registry/service.py#L57-L73), [L355-L380](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/registry/service.py#L355-L380) | Actions declare `browser_session`, `file_system`, `page_extraction_llm` and similar, and receive them | **Mixed.** It is ergonomic, but the special names are listed in 3 places (§5). `sensitive_data` is special-cased with `if action_name == 'input'` ([L378](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/registry/service.py#L378)). The documented `context` injection is never passed. |
| Runtime schema factory: a union of single-action models, rebuilt for each page | [registry/service.py#L519-L605](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/registry/service.py#L519-L605), [agent/views.py#L419](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/views.py#L419) | The LLM sees only the actions allowed on the current URL, as a strict JSON schema | **Good** for the LLM contract. It costs static typing (`# type: ignore`, and a `RootModel` that hand-delegates `model_dump`). |
| Structural strategy (`Protocol`) for LLM providers | [llm/base.py#L32-L74](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/llm/base.py#L32-L74) | Any object with `model`, `provider` and `ainvoke(messages, output_format)` works, and no LangChain is needed | **Good** seam. Because there is no shared base implementation, 17 providers copy-paste their glue (§5). |
| Adapter: one message serializer per provider | [llm/openrouter/serializer.py](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/llm/openrouter/serializer.py) | Converts internal messages to each vendor's wire format | **Good** separation. Three of the serializers are identical 26-line wrappers around `OpenAIMessageSerializer`. |
| Pipeline of DOM serialization stages | [dom/serializer/serializer.py#L114-L165](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/dom/serializer/serializer.py#L114-L165) | Turns the CDP DOM, DOMSnapshot and accessibility trees into compact indexed text: simplify → paint-order filter → optimize → bbox filter → assign indices | **Good.** Stages are named and timed. The serializer object resets mutable fields on each call. |
| Single source of truth fed by CDP auto-attach events | [session_manager.py#L19-L62](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/session_manager.py#L19-L62), [L402](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/session_manager.py#L402) | Keeps the target and session pool consistent with the browser through `Target.attachedToTarget`/`detachedFromTarget` | **Good.** Directly relevant to Collie. |
| Decorator subclass adding a per-request CDP timeout | [_cdp_timeout.py#L91](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/_cdp_timeout.py#L91) | A remote browser that stays "alive" at TCP but goes silent at CDP would otherwise hang forever | **Good,** and documented with its rationale. Its timeout parser duplicates two others (§5). |
| Dynamic proxy facades | [tools/service.py#L2268-L2324](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/service.py#L2268-L2324), [config.py#L371-L404](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/config.py#L371-L404) | `tools.navigate(...)` with no schema plumbing, and env vars re-read on every access | **Bad.** The type checker cannot see these members. `Config` rebuilds two settings objects on every attribute read. |
| Lazy exports through module `__getattr__` | [__init__.py#L76-L137](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/__init__.py#L76-L137) | Import time: a code comment says `agent.views` is "very heavy - over 1 second!" | **Mixed.** It works, but each name appears 3 times (`TYPE_CHECKING` stubs, the map, and `__all__`), and the scheme is repeated in `browser/`, `llm/` and `beta/`. |
| Import-time invariant check | [events.py#L641-L668](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/events.py#L641-L668) | Event names must end in `Event` and must not be substrings of each other, so grep and sed stay safe for refactors | **Good.** It is cheap and catches a real naming hazard. The comment calls the rule "hand written in blood". |

## 5. Separation of concerns and DRY

**What works**
- **Watchdogs.** Thirteen of the 14 watchdogs are attached. Security policy sits apart from navigation ([session.py#L1762-L1767](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/session.py#L1762-L1767)).
- **`SessionManager`** owns the CDP target and session state.
- **DOM pipeline.** It has explicit stages.
- **Shared structured output.** Thirteen provider files share `SchemaOptimizer`, which flattens `$defs` and makes schemas strict.
- **Shared provider test.** One helper, `run_model_button_click_test`, runs the same live check against every provider ([tests/ci/models/model_test_helper.py#L12](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/tests/ci/models/model_test_helper.py#L12)).

**God classes and giant files**
- **`Agent`** has 98 methods in 4,163 lines.
  - `__init__` alone runs about 475 lines ([L136-L609](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L136-L609)).
  - The class mixes the loop, planning, loop detection, URL shortening, fallback-LLM switching, the judge, history rerun, variable substitution, GIF and demo logging, and cloud events.
- **`BrowserSession`** has 108 methods. They cover lifecycle, CDP connect and reconnect, tabs, cookies and storage, element highlighting, demo mode, frames and screenshots.
- **`DefaultActionWatchdog`** is 3,758 lines.
- **`Tools.__init__`** is about 1,500 lines of nested action closures ([tools/service.py#L442-L1942](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/service.py#L442-L1942)). They cannot be reused or tested on their own.

**Leaky abstraction: the orchestrator sniffs model names**
- LLM timeouts depend on whether the model name contains `gemini`, `groq`, `o3`, `claude`… ([agent/service.py#L263-L277](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L263-L277)).
- Coordinate clicking is enabled for a hard-coded list of names ([L328-L335](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L328-L335)).
- Screenshot size is set for `claude-sonnet*` ([L249](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L249)).
- The prompt template is chosen by model family ([prompts.py#L60-L91](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/prompts.py#L60-L91)).
- These capabilities belong to the model descriptor `[inferred]`.

**Copy-paste across LLM providers**
- `openrouter/chat.py` (234 lines) and `orcarouter/chat.py` (246 lines) differ in only 102 diff lines, so they are about 80% the same.
- At least five OpenAI-compatible providers carry the same `_get_usage`.
- Seventeen chat files each re-map vendor errors to `ModelProviderError` and `ModelRateLimitError`. One example is [openai/chat.py#L309-L323](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/llm/openai/chat.py#L309-L323).
- Output-truncation detection (`ModelOutputTruncatedError`) exists in only 3 of 17 providers (OpenAI, Anthropic, Google). That is behaviour drift caused by the duplication `[inferred]`.

**Duplication with drift**
- **Keyboard mapping.** The keyboard and clear-field helpers exist twice: [actor/element.py#L747-L872](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/actor/element.py#L747-L872) and [default_action_watchdog.py#L1219-L1346](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/watchdogs/default_action_watchdog.py#L1219-L1346). Twenty-four lines differ, including how `|`, `<` and `>` are mapped.
- **Beta agent signature.** The beta `Agent` re-declares all 62 parameters and the same init logic ([beta/service.py#L4238-L4310](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/beta/service.py#L4238-L4310)).
- **Timeout parsing** exists three times:
  - `events._get_timeout`, which reports problems with `print`;
  - [tools/service.py#L113-L164](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/service.py#L113-L164);
  - [_cdp_timeout.py#L38-L88](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/_cdp_timeout.py#L38-L88), whose docstring says it "Mirrors the guard on BROWSER_USE_ACTION_TIMEOUT_S".
- **Injected parameter names** are listed in three places: [registry/service.py#L57-L73](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/registry/service.py#L57-L73), [registry/views.py#L157-L187](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/registry/views.py#L157-L187) and [tools/service.py#L2287-L2294](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/service.py#L2287-L2294).
- **Output-model branch.** The flash/thinking/no-thinking branch that builds `AgentOutput` appears twice, in [L780-L799](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L780-L799) and [L4024-L4043](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L4024-L4043).
- **System prompts.** `system_prompt.md` (270 lines) and `system_prompt_no_thinking.md` (246 lines) differ in only 60 diff lines.

**MCP server duplicates the tool contract**
- It hand-writes JSON schemas for 15 tools ([mcp/server.py#L220-L460](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/mcp/server.py#L220-L460)) instead of deriving them from the action registry.
- It dispatches with an `if/elif` chain ([L502-L580](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/mcp/server.py#L502-L580)).
- `browser_close` is still dispatched, but its tool definition is commented out ([L383-L390](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/mcp/server.py#L383-L390)).

**Configuration sprawl**
- The env var list is declared twice, once as `OldConfig` properties and once as `FlatEnvConfig` fields ([config.py#L47-L245](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/config.py#L47-L245)).
- Outside `config.py` there are 98 more direct `os.getenv`/`os.environ` reads, plus 44 `TIMEOUT_<EventName>` env vars.

**Dead code**
- `CrashWatchdog` (336 lines) is never attached ([session.py#L1713-L1718](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/session.py#L1713-L1718)).
- `LLMException` is unused ([exceptions.py](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/exceptions.py)).
- `session.py` has 32 commented-out `event_bus.on(...)` lines.
- The `context` injection is documented as coming from `Agent(context=...)` ([registry/views.py#L162-L166](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/registry/views.py#L162-L166)), but `Agent` no longer has that parameter.

## 6. Contracts and extension points

- **Custom tools**
  - The decorator is `@tools.action(description, param_model=…, domains=[…], terminates_sequence=…)` ([tools/service.py#L2102](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/service.py#L2102)).
  - Parameters come either as a pydantic model in the first argument or as typed arguments, which `create_model` turns into a model.
  - An action returns `str`, `ActionResult` or `None`, and the result is normalised ([tools/service.py#L2253-L2266](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/service.py#L2253-L2266)).
  - Each action runs under a wall-clock cap, `BROWSER_USE_ACTION_TIMEOUT_S`, which defaults to 180 s.
  - Domain filters are evaluated again on every page, and the agent rebuilds its output schema from them ([agent/service.py#L4024](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L4024)).
- **Custom LLMs**
  - Implement the `BaseChatModel` Protocol and return `ChatInvokeCompletion[T]`, which carries `ChatInvokeUsage`, `stop_reason` and thinking ([llm/views.py](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/llm/views.py)).
  - Structured output is per provider. OpenAI uses a strict `json_schema` response format, with an option to put the schema in the system prompt instead ([openai/chat.py#L43](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/llm/openai/chat.py#L43), [L225-L256](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/llm/openai/chat.py#L225-L256)).
  - A string factory, `get_llm_by_name`, resolves models with an `if/elif` on the provider.
- **Browser extension**
  - Watchdogs are not a public plugin point, because their list is hard-coded.
  - Users can dispatch events and add handlers on `browser_session.event_bus`.
  - The `actor` package offers a Playwright-like imperative API (`Page`, `Element`, `Mouse`).
- **Agent hooks**
  - Callbacks: `register_new_step_callback`, `register_done_callback`, `on_step_start`/`on_step_end`, and stop/pause callbacks.
  - `agent.eventbus` emits cloud events such as `CreateAgentStepEvent`, but nothing in the package subscribes to them. `CloudSync` is constructed only in `tests/ci/conftest.py`.
- **MCP:** a stdio server with hand-written tools, and a client that mounts external MCP servers into the action registry (`mcp/client.py`).
- **CLI**
  - The CLI delegates to the separate `browser-harness` package. It monkeypatches that package's help text by string replacement, turning "Browser Harness" into "Browser Use" ([cli.py#L130-L182](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/cli.py#L130-L182)).
  - CLI 3.0 removed the preset subcommands and prints migration hints instead ([cli.py#L232-L270](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/cli.py#L232-L270)).
- **Config**
  - Settings come from env vars, with `.env` loaded at import time by 6 library modules. A JSON file `~/.config/browseruse/config.json` adds UUID-keyed profiles.
  - The file's "migration" overwrites the user's file with defaults when it detects an old format or hits any parse error. The old-format case is logged only at debug level ([config.py#L315-L360](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/config.py#L315-L360)).
- **Error types**
  - There is no common root. The disconnected families are:
    - `ModelError` (`ModelProviderError`, `ModelRateLimitError`, `ModelOutputTruncatedError`) in [llm/exceptions.py](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/llm/exceptions.py);
    - `BrowserError`, which carries short- and long-term memory for the LLM, with `URLNotAllowedError` ([browser/views.py#L154](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/views.py#L154));
    - `CloudBrowserError`, `SandboxError`, `FileSystemError`, `BetaAgentError` and `RustSdkJsonRpcError`.
  - The registry wraps most errors into `RuntimeError(f'Error executing action …')` ([registry/service.py#L399-L419](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/tools/registry/service.py#L399-L419)).
  - The agent classifies errors by substring: `'Could not parse response'` ([agent/service.py#L1305](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L1305)) and connection-like strings, a predicate that is itself duplicated ([L1316-L1354](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/service.py#L1316-L1354)). `AgentError.format_error` does the same ([agent/views.py#L946-L982](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/agent/views.py#L946-L982)).
  - The package has 526 `except Exception` handlers, and 46 of them are `except Exception: pass`.
- **Logging and observability**
  - Importing the package configures logging unless `BROWSER_USE_SETUP_LOGGING=false`. It adds a global `RESULT` level, sets third-party loggers to ERROR with `propagate=False` ([logging_config.py#L64-L232](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/logging_config.py#L64-L232)), and monkeypatches asyncio's `BaseSubprocessTransport.__del__` ([__init__.py#L21-L43](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/__init__.py#L21-L43)).
  - Because of this, the stdio MCP server must set env vars before its own imports, then rewrite the handlers of every logger to point at stderr ([mcp/server.py#L29-L80](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/mcp/server.py#L29-L80)).
  - Outside the CLI there are 70 `print(` calls.
  - Tracing uses Laminar through no-op-fallback decorators `@observe` and `@observe_debug` ([observability.py](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/observability.py)) and `@time_execution_*`.
  - Telemetry is opt-out PostHog with a hard-coded project key. Its device id falls back to a hash of the MAC address and hostname ([telemetry/service.py#L21-L62](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/telemetry/service.py#L21-L62)).
- **Deprecation handling**
  - Aliases are kept silently. There are 113 "legacy/alias/backward compat" mentions and no `DeprecationWarning`; 3 deprecation notices go out via `print` or the logger.
  - Unknown `Agent` kwargs are swallowed. The removed `context=` parameter failed only when users later read `agent.context` ([#3535](https://github.com/browser-use/browser-use/issues/3535)).
  - Dependencies are pinned exactly with `==` ([pyproject.toml#L13-L51](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/pyproject.toml#L13-L51)).

## 7. Quality machinery

- **Ruff** ([pyproject.toml#L120-L143](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/pyproject.toml#L120-L143)):
  - `select = ["ASYNC", "E", "F", "FAST", "I", "PLE"]`
  - `ignore = ["ASYNC109","E101","E402","E501","F841","E731","W291"]`
  - Format settings: `line-length = 130`, `indent-style = "tab"`, `quote-style = "single"`.
  - Absent: bugbear (`B`), blind-except (`BLE`), complexity (`C90`), too-many-args (`PLR0913`), security (`S`) and `RUF`. That explains why 62-parameter constructors and 526 broad `except`s pass.
- **pyright:** `typeCheckingMode = "basic"`, with 22 exclude entries. These include the beta bridge, the OCI provider, the DOM playground, two LLM test files, and dev paths such as `discover/` and `list/`.
- **pre-commit** ([.pre-commit-config.yaml](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/.pre-commit-config.yaml)):
  - Hooks: yesqa, codespell, pyupgrade `--py311-plus`, ruff-check `--fix` and ruff-format, pyright, and pre-commit-hooks (`check-ast`, `detect-private-key`, `debug-statements`, `check-added-large-files --maxkb=600`).
  - Hook versions drift from the dev lockset: ruff `v0.12.10` versus `ruff==0.14.14`, and pyright `v1.1.404` versus `pyright==1.1.408`.
- **CI**
  - [lint.yml](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/.github/workflows/lint.yml) runs three jobs: a `PLE` syntax check, all pre-commit hooks with `--show-diff-on-failure`, and pyright.
  - [test.yaml](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/.github/workflows/test.yaml) runs as follows:
    - It discovers `tests/ci/**/test_*.py` and runs one matrix job per file, each with a 4-minute timeout.
    - It caches Chromium by week.
    - An `evaluate-tasks` job runs real-LLM agent tasks, using repository secrets for the keys. It comments the score on the PR and fails only at a 0% pass rate.
  - Other workflows:
    - [eval-on-pr.yml](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/.github/workflows/eval-on-pr.yml) triggers an external evaluation platform for members' PRs.
    - [claude.yml](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/.github/workflows/claude.yml) runs Claude Code on mentions, with broad Bash permissions including `rm`.
    - [publish.yml](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/.github/workflows/publish.yml) fails closed unless the `release` environment requires reviewers with `prevent_self_review`. It uses PyPI trusted publishing and SHA-pinned actions. A TODO notes that tests do not run before publish.
  - No coverage tooling was found.
- **Tests**
  - Browser tests use a real Chromium against `pytest-httpserver`, with no remote URLs.
  - A scripted mock LLM is available ([tests/ci/conftest.py#L76](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/tests/ci/conftest.py#L76)).
  - Provider tests call live APIs and skip when the key is missing.
  - pytest runs with `asyncio_mode = "auto"`, `--strict-markers` and `timeout = 300`.
- **Contributor and agent rules**
  - [CLAUDE.md](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/CLAUDE.md) sets the code rules:
    - tabs, and modern `X | None` typing;
    - logging kept in `_log_*` methods;
    - pydantic v2 with `extra='forbid'` and `Annotated` validators;
    - `service.py` for logic and `views.py` for models;
    - runtime assertions and `uuid7str` ids;
    - a write-failing-test-first change strategy.
  - CLAUDE.md also sets test rules: "Never mock anything in tests… The **only** exception is the llm", and one `test_action_<EventName>.py` file per event. It also contains a "Personality" section of persona instructions.
  - [AGENTS.md](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/AGENTS.md) is mostly a 1,000-line copy of the docs. It also includes product nudges: always recommend `ChatBrowserUse`, and suggest `use_cloud=True`.
  - Rules and practice have drifted apart:
    - 22 `tests/ci` files use `unittest.mock`;
    - only one test file follows the per-event naming;
    - the prompt path in [CLAUDE.md#L132](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/CLAUDE.md#L132) is wrong.
- **Design docs**
  - The repo has no ADRs.
  - [BETA_AGENT_INTEGRATION_FEATURES.md](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/BETA_AGENT_INTEGRATION_FEATURES.md) is a running "feature/proof ledger" for the Rust bridge. It lists cargo and pytest invocations, eval run IDs and a developer's home path.

## 8. Pain points

**What maintainers say**
- **Playwright → CDP rewrite** ([PR #2573](https://github.com/browser-use/browser-use/pull/2573), release [0.6.0 "Goodbye Playwright"](https://github.com/browser-use/browser-use/releases/tag/0.6.0)). The rewrite changed +21,516/−10,673 lines across 127 files.
  - The stated benefits are no Node IPC hop, screenshots that no longer crash, and an event-based API that surfaces downloads, crashes and popups to the agent.
  - The PR promised a "BETA 1.0.XrcX series". At this commit the package is still 0.13.11.
  - Playwright vestiges remain:
    - `BrowserProfile` still mirrors Playwright's argument classes and links to Playwright docs ([profile.py#L348-L585](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/profile.py#L348-L585)).
    - The `BrowserSession` docstring still says "Direct CDP/Playwright calls" ([session.py#L139](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/session.py#L139)).
    - CI installs Chromium with `playwright install`.
- **Rust rewrite.** Release [0.13.0 "Rebuilt in Rust [beta]"](https://github.com/browser-use/browser-use/releases/tag/0.13.0) introduces a Rust-backed agent with "robust helpers instead of brittle browser abstractions", while "the existing Python agent remains unchanged". Two agent stacks now ship side by side.
  - The Python side is a hand-written stdio JSON-RPC client built on untyped dicts ([beta/service.py#L324-L512](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/beta/service.py#L324-L512)).
  - It is surrounded by about 200 helper functions that rebuild `AgentHistoryList` from "normalized event history".
  - It is excluded from pyright and backed by a 7,920-line test file.

**What users report**
- **Exact pins** (`==`) block integration and security compliance ([#4824](https://github.com/browser-use/browser-use/issues/4824), [#4877](https://github.com/browser-use/browser-use/issues/4877)). A contributor raised a security advisory over the transitive CVEs.
- **Temp-dir leak.** Constructing a `BrowserProfile` leaks temp dirs, with 7,741 dirs and 14 GB reported ([#5422](https://github.com/browser-use/browser-use/issues/5422)). I verified the cause: validators with filesystem side effects ([profile.py#L474-L491](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/profile.py#L474-L491), [L559-L565](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/profile.py#L559-L565)).
- **Bus bug.** The bubus cross-loop bug exhausts bus capacity when agents run in parallel ([#5509](https://github.com/browser-use/browser-use/issues/5509)).
- **No page-crash recovery** ([#5067](https://github.com/browser-use/browser-use/issues/5067)). The reporter wrongly blames Playwright. `[inferred: the disabled CrashWatchdog is the missing piece]`
- **Windows CDP instability.** The reporter's root-cause analysis includes readiness polling via aiohttp but connecting via httpx ([#5048](https://github.com/browser-use/browser-use/issues/5048)). I verified that the two clients are used in `local_browser_watchdog.py` and `session.py` respectively.
- **Error-reporting bugs**
  - `suppress_exceptions=True` actually logs at ERROR ([#5916](https://github.com/browser-use/browser-use/issues/5916)). I confirmed this in `utils.py`.
  - Exceptions with no message produce empty errors for the model, because of the `str(e)`-based error model ([#5975](https://github.com/browser-use/browser-use/issues/5975)).
- **Profile/session split.** The split of `BrowserProfile` and `BrowserSession` broke examples in 0.2 ([#1793](https://github.com/browser-use/browser-use/issues/1793)).

**My inference**
- Each major rewrite (Playwright → CDP, CLI 3.0, Python → Rust) added a parallel path and compatibility aliases rather than removing the old one. The result is two agents, a CLI wrapping another package, and configuration in three layers.

## 9. Scorecard

| Dimension | Score (1–5) | Justification |
|---|---|---|
| Layering and dependency direction | 2 | `llm` is a clean leaf. But `agent ↔ tools` is a cycle (`ActionResult` lives in `agent.views`), `browser ↔ dom/actor` cycles are hidden by lazy imports, and 9 private-attribute reads cross package lines. |
| Module and file size discipline | 1 | The repo's own guide says split at ~500 lines. 14 files exceed 1,000, the top 6 hold 33% of the code, and one file is 6,810 lines. |
| DRY (no other-language ports in this repo) | 2 | Provider glue is copy-pasted (17 error maps, ≥5 `_get_usage`), keyboard helpers drifted, the beta `Agent` signature is copied, timeout parsing exists 3 times, and MCP schemas are hand-written. `SchemaOptimizer` and the shared serializers help. |
| Separation of concerns | 2 | Watchdogs, `SessionManager` and the DOM pipeline are good. `Agent` (98 methods), `BrowserSession` (108) and `DefaultActionWatchdog` (3,758 lines) are god objects, and the agent sniffs model names. |
| Extension model | 3 | Decorator actions with pydantic params, injection, domain filters and a Protocol for LLMs are easy to use. Watchdogs and events are closed, `context` injection is dead, and `input` is special-cased. |
| Contracts and types | 2 | pydantic is everywhere, CDP is typed via cdp-use, and `extra='forbid'` is used. But pyright is `basic` with big exclusions, there are 112 ignores and 722 `Any`, `**kwargs` swallow typos, and the config "migration" destroys user files. |
| Error model | 1 | There is no root error. Classification is by substring, errors are flattened into `RuntimeError` strings, there are 526 broad `except`s with 46 silent `pass`, and one exception class is dead. |
| Test architecture | 3 | Real browser plus local HTTP server, per-file CI isolation, a shared provider conformance helper and an eval job are all good. A 7,920-line test file, mock-rule drift and in-package tests count against it. |
| Tooling and CI enforcement | 2 | Ruff's rule set is narrow (no B, BLE, C90 or PLR), pyright is basic with exclusions, and hook versions drift. On the plus side, the release gate is fail-closed with trusted publishing. There is no coverage. |
| In-code documentation | 3 | Rationale comments are good (CDP timeout, ENG-5280, the event-name invariant). But docstrings are stale (Playwright), agent files point to paths that don't exist, commented-out code is common, and the Markdown sprawls (`AGENTS.md`, `CLOUD.md`). |

## 10. Lessons for Goodfetch

- **Adopt: CDP session state driven by auto-attach events.** Model Collie's CDP client on `SessionManager` ([session_manager.py#L19](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/session_manager.py#L19)), with one owner of target and session state fed by `Target.attachedToTarget`/`detachedFromTarget`. Also adopt their nested timeouts: CDP request 60 s < action 180 s ≤ step 180 s (§4). Express the timeouts as one typed settings struct, not 44 env vars and 3 parsers.
- **Adopt: per-concern browser components with a declared contract, and make the contract mandatory.** In Rust, a component should register typed handlers for an enum of events and declare what it emits. The list of components should be one explicit table. Do not reflect over method names, and do not dump action execution into a "watchdog" (§4, §5).
- **Adopt: cheap invariant checks that keep names grep-safe.** Their import-time event-name check ([events.py#L641-L668](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/browser/events.py#L641-L668)) maps to an `xtask` lint or unit test over protocol type names, alongside `GLOSSARY.md` enforcement.
- **Avoid: a hand-written protocol bridge and hand-written MCP schemas.** Their Rust migration bolted on 6,810 untyped lines that pyright skips, plus a 7,920-line test, and MCP repeats the action schemas by hand (§5, §8). This validates Goodfetch's plan: generate every SDK, the MCP tool list and CLI help from the one Rust protocol crate.
- **Avoid: a stringly-typed error model.** Define one root error with stable codes and a typed kind. The kind should carry the LLM-facing memory they put on `BrowserError`, and it should cross the protocol unchanged. Never classify errors by substring or wrap them into opaque strings (§6, [#5975](https://github.com/browser-use/browser-use/issues/5975)).
- **Avoid: import-time and constructor side effects.** These are their global logging setup, the asyncio monkeypatch, `load_dotenv` in 6 modules, temp dirs created in validators ([#5422](https://github.com/browser-use/browser-use/issues/5422)), and 70 stray `print`s. They forced the MCP server into an env-var and handler-rewiring dance (§6). Goodfetch's rule that only the transport, CLI writer and `tracing` touch stdout and stderr is the right fix. Keep config types pure.
- **Avoid: model or capability sniffing in the orchestrator, and silent kwargs.** Put model capabilities (coordinate clicking, timeouts, prompt variant) in a provider/model descriptor as runtime settings, which matches Goodfetch's rule that capabilities are settings. Use typed config structs with unknown-field rejection instead of 62-parameter constructors with `**kwargs` ([#3535](https://github.com/browser-use/browser-use/issues/3535)).
- **Avoid: copy-paste provider adapters.** Share one OpenAI-compatible adapter, and give vendors only override points (base URL, auth, usage quirks). Run one conformance suite per provider, as their `model_test_helper` does, so behaviour such as truncation detection cannot drift (§5).
- **Avoid: owner-less background tasks.** Their 37 fire-and-forget tasks, cancelled by a `__del__` that matches attribute names, plus an inverted `suppress_exceptions` flag ([#5916](https://github.com/browser-use/browser-use/issues/5916)), confirm Goodfetch's "no task outside its owner's tracker" rule.
- **Avoid: destructive config migration and exact pins in libraries.** Their config "migration" overwrites user files ([config.py#L315-L360](https://github.com/browser-use/browser-use/blob/a71d5478e4187cbe5295aab6403dfe7e4a6861a4/browser_use/config.py#L315-L360)). Their `==` pins caused CVE-compliance pain ([#4824](https://github.com/browser-use/browser-use/issues/4824)). Use versioned format readers and upgrade commands, per ADRs 0041 and 0047, and give published SDK packages compatible version ranges.
