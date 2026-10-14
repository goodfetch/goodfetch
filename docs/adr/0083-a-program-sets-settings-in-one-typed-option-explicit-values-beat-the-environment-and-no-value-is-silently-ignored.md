# A program sets settings in one typed option, explicit values beat the environment, and no value is silently ignored

ADR 0028 makes every parameter a setting and resolves it through five layers; this decides how a program reaches them. Each value that opens a scope takes one `settings` option, typed from the settings schema in each language's own form, while a call takes only its task's inputs and `timeout`. A value written in code beats the environment, which carries only a closed list of deployment variables. A value that cannot take effect where it is set is refused when the scope opens, never ignored, clamped or dropped. Configuration is the research's fifth crawl pattern and the AI tools' sixteenth pain: Browser Use swallows unknown keyword arguments, reads the environment at import and lets an empty variable switch telemetry on; Playwright ignored `isVisible({ timeout })`; Crawlee v3 let environment variables override its constructor until v4 reversed it ([consumer API research](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/2026-10-14-consumer-api.md), its [AI](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/ai-browser-automation.md) and [crawl](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/crawl-frameworks.md) notes; [SDK design guidelines research](https://github.com/goodfetch/automan/blob/research/sdk-design-guidelines/rnd/research/2026-10-14-sdk-design-guidelines.md), items 26 to 29). Decided in [Settings in code](https://github.com/goodfetch/automan/issues/470), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461).

## Decision

### Precedence

- **Explicit beats the environment, which beats configuration files.** Within ADR 0028's run layer, an explicit value (a call's argument, a `settings` option, a CLI flag, an Integration's form) beats an environment variable, which only fills what was left unset. Stainless SDKs read `OPENAI_API_KEY` only as a default an argument replaces, Azure requires per-client configuration to override every global source, and Crawlee v4 reversed v3's order for the same reason. What a reader sees in the code is what runs (G2, G6).
- **A plain program's values are the run layer.** ADR 0028's layer 4, "a Recipe or the developer's code", means a Recipe's code, whose declared settings are locked for the Recipe's users unless exposed. A program that is not a Recipe has no separate users, so its values sit in the run layer, above configuration.
- **The layer is named.** An error's `limit` (ADR 0079) and `explainSettings()` name the layer, and for the run layer its source: a call's argument, `settings`, a flag, a form or the environment.

### The environment

- **A closed list of deployment variables**, the values that differ by machine rather than by program: `GOODFETCH_HOME`, `GOODFETCH_ENGINE_PATH` and `GOODFETCH_BROWSERS_PATH` (ADR 0024), `GOODFETCH_URL` (ADR 0049) and `GOODFETCH_TOKEN`, the scoped token it needs, and the test variables of ADR 0062's test support, such as `GOODFETCH_TEST_SEED`. Each is a setting in the generated schema, marked as read from the environment. No other setting has an environment variable: a variable per setting would let any shell or CI job reshape a crawl unseen (G2, G5), and the research's lesson is "env vars only for secrets and deployment knobs". Secrets keep their own route, the Secret reference `env:NAME`, which names any variable the user chooses (ADR 0015).
- **The engine choice.** `GOODFETCH_URL` fills ADR 0078's `engine` option only when the code leaves it at `auto`; there is no `GOODFETCH_ENGINE`. `GOODFETCH_TOKEN`'s value never appears in a log, an error or `explainSettings()`, which show it as set; whether it also takes a Secret reference is [Sessions, logins, Human pause and secrets in code](https://github.com/goodfetch/automan/issues/476)'s.
- **Read once, strictly.** `connect()` reads the environment once, from the environment map of ADRs 0062 and 0069, which an option replaces and which defaults to the process's own, in all three SDKs (S1): the Rust crate reads the process environment there and nowhere else, unlike Azure's Rust rule, because a deployment variable must mean the same in every SDK and Goodfetch targets no WASM platform. Nothing is read at import (G4). The `GOODFETCH_` prefix is reserved (ADR 0034): an unknown name fails `connect()` with `setting_unknown`, naming the closest listed variable, and an empty or unparsable value with `setting_invalid`. A newer tool's variable in an older SDK's environment fails too; ADR 0030's single product version makes that rare, and failing beats Browser Use's empty variable that switched telemetry on.
- **Whose environment.** The list governs an SDK program, the environment an Attached engine is given, and the CLI's run layer, where flags beat it. A Daemon's own process configuration (ADR 0049) is the operator's, outside this decision.

### Settings in code

- **One `settings` option on each value that opens a scope**: `connect()`, starting a Run, opening a Session and opening a page. It holds any setting that applies within that scope; the nearest scope wins, and values are inherited by everything opened beneath it. A call takes only its task's inputs and `timeout` (ADR 0080). The leaders split the other way and are criticized for it: Browser Use's `Agent` has about 60 keyword arguments, Crawlee spreads constructor options beside a global `Configuration` (S2).
- **A per-call argument** may exist for a setting that is part of one call's task, as `timeout` is (ADR 0080), such as a model chosen for one extraction; the verb's own ticket decides it. The argument applies to that call alone and is that setting at the run layer.
- **No setters.** Nothing changes a setting of a scope already open, as Playwright's `page.setDefaultTimeout` does (G2).
- **The options of `connect()` that are not settings stay its own:** `engine` and `logger` (ADR 0078), `timeout` and `limits` (ADR 0080), the environment map, `project` (below) and `settings`.
- **Three states.** A setting left out inherits; `null` (`None` in Python and Rust) clears it where its type allows, such as no timeout; a value sets it. The SDK sends only what the program set and validates nothing the engine owns, so an Operator policy is never bypassed by an SDK-side default (guidelines items 26 and 27).

### Each language's form

Generated from the settings schema (ADR 0028), one form throughout each SDK (S1):

- **TypeScript: a nested object literal**, each segment of the key camelCased, typed under `exactOptionalPropertyTypes`, so an unknown key in a literal is a compile error: `settings: { crawl: { payload: { maxSize: 65_536 } }, timeout: { navigation: 60_000 } }`. It is Playwright's option style (S3); flat dotted strings would match configuration files but complete worse and read unlike TypeScript.
- **Python: a nested `TypedDict` literal**, every key `NotRequired`, which pyright and mypy check key by key: `settings={"crawl": {"payload": {"max_size": 65_536}}, "timeout": {"navigation": timedelta(seconds=60)}}`. openai-python and anthropic-python type their nested request parameters the same way (S3); generated dataclasses would need a class import per level and a `not_given` default on every field, and pydantic-settings' `crawl__payload__max_size=` is a second spelling of every key.
- **Rust: typed key constants on a `Settings` value**: `Settings::new().set(crawl::payload::MAX_SIZE, 64 * 1024).set(timeout::NAVIGATION, Some(Duration::from_secs(60)))`. Each key is a generated `Setting<T>` constant in a module path mirroring its dotted name, so the value's type is checked; a setting that can be cleared has `T = Option<…>`, and `None` clears it. It scales to hundreds of keys without as many builder methods, as `http::header::CONTENT_TYPE` does; nested closure builders read poorly, and nested structs could not be `#[non_exhaustive]` and would need `Option<Option<T>>`.

The same nesting reads the same in TypeScript, Python, TOML and YAML configuration, which helps a coding agent move between them (T).

### Names

- **One canonical key per setting**, dotted snake_case with its area first, as in `crawl.payload.max_size` (ADR 0082) and `session.source` (ADR 0056). Configuration files, Recipe manifests, `goodfetch config explain`, an error's `limit` and the CLI's references use it verbatim; TypeScript camelCases each segment, Python keeps it, and Rust spells each segment as a module and the last as an UPPER_SNAKE constant. The listed environment variables have fixed names. [Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471) applies the same casing to operations; the CLI's flag syntax is the CLI's own design.
- **ADR 0080's timeouts are one family**: `timeout.connect`, `timeout.action`, `timeout.navigation`, `timeout.wait`, `timeout.handler`, `timeout.hook`, `timeout.sink_write`, `timeout.close`, and `timeout.run` for the Run's own. Playwright Test names `actionTimeout` and `navigationTimeout`, Crawlee `navigationTimeoutSecs` and `requestHandlerTimeoutSecs`; one family gives one place to look (`goodfetch config explain timeout`) and a `limit` that reads `timeout.navigation` (S2, G6).
- **One built-in default per setting, the same in Collie and Foxhound.** A crawl's longer needs have crawl-only settings, such as `timeout.handler`, which also bounds every action inside the handler because timeouts nest; product-dependent defaults would make the same `click` behave differently by which product opened the page, and `config explain` show two columns (S2, G6).

### Refusals

Checked when the scope opens, or when a call with a per-call argument starts, so the error points at the line that set the value (G6). Nothing is clamped, dropped or ignored (G3), unlike Playwright's `isVisible({ timeout })` and Skyvern's `webhook_url`:

| Case | Error code | Group |
|---|---|---|
| An unknown key, or an unknown `GOODFETCH_` variable; the message names the closest valid one | `setting_unknown` | `ProgramError` |
| A value of the wrong type or out of the setting's own range | `setting_invalid` | `ProgramError` |
| A setting no scope beneath can use, such as a `crawl.*` setting on a Collie Run or `browser.viewport` on a page | `setting_not_applicable` | `ProgramError` |
| A value a lower layer locked or capped, or outside a Recipe's exposed bounds; `limit` names that layer and its value | `setting_locked` | `RefusedError` |
| A capability the platform cannot provide; its fix names the setting that turns it off | `capability_unavailable` | `UnavailableError` |

A capability is a setting, never a Cargo feature (ADR 0038), and one binary carries every capability (ADR 0057), so the types are the same in every build: Spider's 132 Cargo features change field types, and its default `io_uring` panicked in Docker.

### Reading settings

Each scope value has one read-only call, `explainSettings(keys?)` (`explain_settings` in Python and Rust), returning what `goodfetch config explain` shows for that scope: per key, the effective value, the layer and source that set it, and the layer that locked it. A program or a test can assert what it actually runs with (golden scenario 10), as Scrapy spiders read `self.settings` and Crawlee calls `Configuration.get`, without a setter.

### Project configuration

`connect()` reads project configuration as the CLI does, searching upward from the working directory, as git and Scrapy's `scrapy.cfg` do, so `goodfetch config explain` run in the same folder shows what the program gets. A `project` option names another file, or `null` reads none, for a service whose working directory means nothing (golden scenario 12; Azure's per-client opt-out). The SDK sends the file's values as ADR 0028's layer 3, which works with a remote Daemon too, and `gf.engine` reports the file used, also logged at info (G6).

### Browser and context setup

- **Session settings under `browser.*`, with Playwright's names** where the meaning matches (S3): `browser.viewport`, `browser.device_scale_factor`, `browser.locale`, `browser.timezone_id`, `browser.color_scheme`, `browser.java_script_enabled`, `browser.geolocation`, and `network.throttle`, which Playwright closed as not planned ([#8622](https://github.com/microsoft/playwright/issues/8622), +94).
- **Set where a Session opens, or above it as a default**, never per page and never changed mid-Session: they are part of the Session's identity, fixed for its life (ADR 0021), so Playwright's `setViewportSize` has no counterpart (G2).
- **Locale and time zone** default to ADR 0021's match with the proxy exit; an explicit value replaces the match, and `config explain` and the Run trace show it. In the `stealth` Access mode the Identity profile owns them (ADR 0035), and a conflicting value is refused with `setting_locked`.
- Where contexts and pages are opened is [Collie's page API](https://github.com/goodfetch/automan/issues/472)'s.

## Considered Options

- **Environment variables above code**, ADR 0028's literal reading and Crawlee v3's order: rejected. A variable set in a shell or a CI job silently overrides what the program says, which Crawlee v4 reversed.
- **An environment variable per setting**, generated as crawlee-python's pydantic-settings `Configuration` does: rejected. Every setting gets a second name, and any shell can reshape a crawl unseen (G2, G5).
- **No environment at all**, as Azure's Rust rule: rejected. It breaks ADR 0049's `GOODFETCH_URL` and the test fixtures of ADR 0069.
- **Ignoring unknown `GOODFETCH_` variables, or failing only on near misses**: rejected. One strict rule is easier to state and to trust (G3), and ADR 0077 rejected lenient inputs with warnings.
- **Rust reading the environment only on request**: rejected. `GOODFETCH_URL` would work in two SDKs and do nothing in the third (S1).
- **Every setting as a top-level option of each opener**, as Browser Use's `Agent` and Crawlee's constructors: rejected by S2; signatures would grow to hundreds of fields.
- **A separately built `Settings` value in TypeScript and Python**: rejected. An extra type to learn, and a pull toward one shared global instance (G2).
- **Flat dotted string keys in TypeScript and Python**: rejected. They read unlike either language (S1) and complete worse; the canonical key stays what configuration files and errors show.
- **Generated dataclasses, or double-underscore keywords, in Python**: rejected above, under Each language's form.
- **Nested builders or nested structs in Rust**: rejected above, under Each language's form.
- **Timeouts named under their subjects** (`page.navigation.timeout`, `crawl.handler.timeout`): rejected. They scatter one concept of ADR 0080 across areas.
- **Product-dependent defaults** (Crawlee's 60 s navigation for crawls, Playwright's 30 s for scripts): rejected. Nesting already bounds a crawl's actions by its handler's timeout.
- **Project configuration read only when named, or never**: rejected. A program and the CLI in the same folder would disagree about what runs.
- **Clamping a value above a cap, or applying it where it has no effect**: rejected by G3; a silent no-op is how Playwright's ignored `timeout` and Skyvern's ignored `webhook_url` hurt.
- **Viewport per page**, as Playwright's `newPage({ viewport })` and `setViewportSize`: rejected. A Session's identity is fixed for its life (ADR 0021).

## Consequences

- **Amends ADR 0028.** The run layer orders its sources: explicit values (arguments, `settings`, flags, forms) above environment variables; layer 4's "developer's code" is a Recipe's code; the environment carries only the deployment variables above.
- **Amends ADR 0080.** `.no_timeout()` is the one spelling of no timeout for a call and the option builders, while a scope's timeout setting is cleared with `Settings::set(timeout::…, None)` in Rust. The setting names are those above, and Foxhound's defaults are Collie's.
- **Amends ADR 0021.** An explicit `browser.locale` or `browser.timezone_id` replaces the `standard` Access mode's match with the proxy exit, and is shown.
- **Amends ADR 0079.** `limit` names the run layer's source, including an environment variable.
- **Answers ADR 0078's open point.** The environment sets the engine only through `GOODFETCH_URL`, and the connection's other settings go in its `settings` option.
- **Error codes.** `setting_unknown`, `setting_invalid` and `setting_not_applicable` join `goodfetch-protocol` in the `program` group, `setting_locked` in the `refused` group and `capability_unavailable` in the `unavailable` group, through `xtask codegen`'s naming checks (ADR 0079).
- **The settings schema** (ADR 0028) gains, per setting, its canonical key, the scopes where it applies, whether it can be cleared, and whether the environment may carry it; `xtask codegen` emits from it the TypeScript object types, the Python `TypedDict`s and the Rust key constants.
- **Area 3 (Engine runtime)** builds the resolver's run-layer order, the strict environment, the refusals and `explainSettings`. **Area 5 (Site access)** builds the `browser.*` settings and `network.throttle`. **Area 11 (Client surfaces)** builds the three forms, the environment map's reading and project discovery.
- **ADR 0037's matrix**: typed configuration and environment variables (RT-22), the timeouts' names (RT-06) and device emulation (SA-23) cite this ADR.
- **The `prototype/sdk-api` sketch** holds these signatures in its `settings` files for each language; the pain coverage matrix closes P27 as Elsewhere (ADRs 0038, 0057), P38 as API and P54's Settings part as API.
