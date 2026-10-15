# SDK catalog mapping: external facts check

**Question.** Which external conventions and limits can the mapping from the Operation catalog to each SDK API, to deprecation and experimental markers, and to agent tool names rely on? This note checks 27 claims (A1 to D6).

**Date read.** Every source below was read on 2026-10-15.

**Method.** Primary sources only. Source files at a pinned tag or commit, fetched with `gh api`. Official docs and specs. docs.rs source views for generated Rust crates. Some behaviours were also checked by running local tools: rustc 1.93.0 (B3, B7, C5), tsc 5.9.3 (C4), and mypy 1.19.1 with pydantic 2.12.4 (C3, C6). Line numbers refer to the pinned revision. Statuses are VERIFIED, REFUTED, PARTLY or UNVERIFIED.

## A. Playwright

**A1. VERIFIED.** At v1.64.1, `docs/src/api` holds 55 files with **461 `* langs:` lines** (grep count). The densest files are `params.md` (93), `class-page.md` (88) and `class-locatorassertions.md` (74).

- **How the lines split.** 66 are bare `* langs:` lines that only carry aliases. The others limit a member to some languages: `js` 89, `python` 85, `js, python` 38, `csharp, java` 35, and so on.
- **Alias counts.** `alias-java` 52, `alias-python` 46, `alias-csharp` 42, `alias-js` 16.
- **The `waitForResponse` example.** `Page.waitForResponse` carries `alias-python: expect_response` and `alias-csharp: RunAndWaitForResponse`. A second entry with the same name, marked `langs: python`, returns `EventContextManager<Response>`.

Sources: [class-page.md L5484-L5567](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-page.md#L5484-L5567), [docs/src/api](https://github.com/microsoft/playwright/tree/v1.64.1/docs/src/api).

**A2. VERIFIED.** `scripts/codegen/python_rules.js` sits next to `generate_api.js` and `compare_api.py`.

- **Origin.** It was added by PR #3240, "generate the public API from the upstream docs", merged 2026-10-12T21:49Z (merge commit `b93d8b94f2`). PR #3247 extended it the next day for TypedDicts.
- **Stated policy.** "the shipped Python API is the reference". Each entry records one place where Python differs from the docs. "The preferred fix is a docs change upstream." Once the docs are fixed, the generator reports the entry as unused, and it must be removed. Every entry needs a one-line reason.
- **Rule tables.** `extraClasses`, `handWrittenClasses`, `contextManagerClasses`, `extraMembers`, `extraOptions`, `skipArgs`, `argNames`, `typeOverrides`, `positional`, `positionalOptions` and `structRenames`.
- **Property rule.** `generate_api.js` turns a sync docs method with no arguments into a Python property. The exceptions are names starting with `is_` or `as_`, and `connect_to_server`.

Sources: [python_rules.js L19-L27](https://github.com/microsoft/playwright-python/blob/v1.64.0/scripts/codegen/python_rules.js#L19-L27), [generate_api.js L152-L221](https://github.com/microsoft/playwright-python/blob/v1.64.0/scripts/codegen/generate_api.js#L152-L221), [PR #3240](https://github.com/microsoft/playwright-python/pull/3240).

**A3. VERIFIED.** In playwright-python v1.64.0, `playwright/async_api/_generated.py` has:

- **Methods.** `Route.continue_` (L22298), `Locator.and_` (L10652) and `Locator.or_` (L12764).
- **Properties.** `Response.ok` (L22167), `Response.status` (L22215) and `Page.url` (L20799) carry `@property`.
- **The `is_closed` method.** `Page.is_closed()` is a plain method (L18593).
- **Classes.** `CDPSession` (L4556), `APIRequestContext` (L215) and `JSHandle` (L5365).

**Where the names come from:**

- **Keyword aliases.** The docs supply them. `Route.continue` has `alias-python: continue_` and `alias-java: resume`. `Locator.and` and `Locator.or` have `alias-python: and_` and `or_`.
- **Property or method.** The generator rule in A2 decides this, not a per-member annotation.
- **Snake case.** The conversion is mechanical: `newCDPSession` becomes `new_cdp_session`, `setExtraHTTPHeaders` becomes `set_extra_http_headers`, `ignoreHTTPSErrors` becomes `ignore_https_errors` and `baseURL` becomes `base_url`.

Sources: [_generated.py](https://github.com/microsoft/playwright-python/blob/v1.64.0/playwright/async_api/_generated.py#L22160-L22260), [class-route.md L36-L40](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-route.md#L36-L40), [class-locator.md L115-L118](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-locator.md#L115-L118).

**A4. VERIFIED.** All lines below are in `packages/playwright-core/types/types.d.ts` at v1.64.1.

- **Methods and the `CDPSession` class.**
  - `waitForURL`: Page L5792, Frame L9137.
  - `newCDPSession(page)`: L10384, returning `CDPSession` (interface at L18406).
  - `setExtraHTTPHeaders`: L4876 and L10598.
- **Options.** `baseURL?` (L11328), `bypassCSP?` (L11333), `ignoreHTTPSErrors?` (L11499).
- **Accessors are methods.** `Response.ok(): boolean` (L22974), `Response.url()` (L23043), `Request.url()` (L22886).
- **Page events.** `domcontentloaded` (L1061), `filechooser` (L1082), `pageerror` (L1118), `requestfailed` (L1173), `requestfinished` (L1179), `websocket` (L1190).

Nuance: the docs spell these events in camelCase, for example `Page.requestFinished`, `Page.DOMContentLoaded` and `Page.webSocket` (class-page.md L539, L328, L553). The generators lower-case them, and Python uses the same lowercase literals. Source: [types.d.ts](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/types/types.d.ts#L1061-L1190).

**A5. PARTLY.**

- **The rename is real.** `Page.goto` and `Frame.goto` both carry `alias-java: navigate`: [class-page.md L2539-L2542](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-page.md#L2539-L2542), [class-frame.md L1144-L1147](https://github.com/microsoft/playwright/blob/v1.64.1/docs/src/api/class-frame.md#L1144-L1147).
- **`goto` is reserved in Java.** JLS §3.9: "The keywords const and goto are reserved, even though they are not currently used." ([JLS SE 25 §3.9](https://docs.oracle.com/javase/specs/jls/se25/html/jls-3.html#jls-3.9)).
- **The stated reason was not found.** No Playwright source, commit or issue found states that the keyword is the reason. Searched: microsoft/playwright and microsoft/playwright-java code, commits and issues. The Java generator has no keyword table; it only applies the docs aliases ([ApiGenerator.java](https://github.com/microsoft/playwright-java/blob/v1.63.0/tools/api-generator/src/main/java/com/microsoft/playwright/tools/ApiGenerator.java)).

The reason is a strong inference. In the same way, Java renames `Route.continue` to `resume`.

**A6. VERIFIED.**

- **The protocol enum.** `SDKLanguage` (javascript, python, java, csharp) is defined in [protocol/spec/core.yml L36-L42](https://github.com/microsoft/playwright/blob/v1.64.1/packages/protocol/spec/core.yml#L36-L42). The client sends it in `Playwright.initialize` ([playwright.yml L20-L23](https://github.com/microsoft/playwright/blob/v1.64.1/packages/protocol/spec/playwright.yml#L20-L23)).
- **Locators.** The server renders them with `asLocator(this._page.browserContext._browser.sdkLanguage(), selector)` ([server/frames.ts L1866](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/frames.ts#L1866)).
- **Errors.** It does the same in error messages, for example "Frame locators are not allowed inside composite locators" ([frameSelectors.ts L129-L134](https://github.com/microsoft/playwright/blob/v1.64.1/packages/playwright-core/src/server/frameSelectors.ts#L129-L134)).
- **Traces.** The context event records `sdkLanguage` ([trace/versions/traceV10.ts L98](https://github.com/microsoft/playwright/blob/v1.64.1/packages/isomorphic/trace/versions/traceV10.ts#L98)). The trace model reads it (traceModel.ts L103). The viewer renders action titles and locators with it ([callTab.tsx L44, L120](https://github.com/microsoft/playwright/blob/v1.64.1/packages/trace-viewer/src/ui/callTab.tsx#L44-L120)).

Nuance: in code the TypeScript type is `Language` (locatorGenerators.ts L23), which adds `jsonl`. When the language is unknown, rendering falls back to `javascript`.

## B. Naming conventions

**B1. VERIFIED.** PEP 8, at python/peps `5514795`:

- **Keywords.** `single_trailing_underscore_` is "used by convention to avoid conflicts with Python keyword" (L960-L961). Arguments (L1077-L1081) and public attributes (L1139-L1141) get the same advice: a trailing underscore beats an abbreviation.
- **Acronyms.** "When using acronyms in CapWords, capitalize all the letters of the acronym." "Thus HTTPServerError is better than HttpServerError." (L932-L934).

Source: [pep-0008.rst](https://github.com/python/peps/blob/5514795ade79a5bf6f3c08c562b02689d0a9feb2/peps/pep-0008.rst#L932-L934).

**B2. VERIFIED.** Rust API Guidelines, C-CASE: in `UpperCamelCase`, "acronyms and contractions of compound words count as one word". The examples are `Uuid` not `UUID`, `Usize` not `USize`, and `Stdin` not `StdIn`. In `snake_case` they are lower-cased (`is_xid_start`). Source: [naming.md L31](https://github.com/rust-lang/api-guidelines/blob/19f0f62ecd0f56a0b7adaa332c352bc2476506ea/src/naming.md#L31).

**B3. VERIFIED.** On `s.type()`, rustc reports "expected identifier, found keyword `type`" and offers help: "escape `type` to use it as an identifier", with the fix `s.r#type()`. Reproduced with local rustc 1.93.0.

- **In source.** The suggestion is defined as `#[suggestion("escape `{$ident_name}` to use it as an identifier", code = "r#")]` ([rustc_parse/src/diagnostics.rs L1464-L1469](https://github.com/rust-lang/rust/blob/7df81381d027710708209f36b98d11b1c88f8d2e/compiler/rustc_parse/src/diagnostics.rs#L1464-L1469)).
- **In tests.** A UI test fixes `m::for()` to `m::r#for()` ([issue-57198.stderr](https://github.com/rust-lang/rust/blob/7df81381d027710708209f36b98d11b1c88f8d2e/tests/ui/parser/issues/issue-57198.stderr)).

**B4. VERIFIED (nuances).**

- **prost.** In prost-build v0.14.4, `sanitize_identifier` turns Rust keywords such as `type` into `r#type` ([ident.rs L5-L25](https://github.com/tokio-rs/prost/blob/v0.14.4/prost-build/src/ident.rs#L5-L25)). `self`, `super`, `crate`, `Self`, `extern` and `_` get a trailing `_` instead, since most of these cannot be raw identifiers.
- **aws-sdk-route53 1.127.0.** `types::ResourceRecordSet` has `pub r#type: RrType` and `pub fn r#type(&self)` (source L49 and L275). Its builder has `r#type()`, `set_type()` and `get_type()` (L597, L627, L657): the prefixed forms drop `r#`. Source: [_resource_record_set.rs](https://docs.rs/aws-sdk-route53/1.127.0/src/aws_sdk_route53/types/_resource_record_set.rs.html).
  - Caveat: the rendered rustdoc page shows the method as plain `type` (anchor `method.type`), so read the source view.
- **playwright-rs v0.20.0.** It spells `Route::continue_()` ([route.rs L155](https://github.com/padamson/playwright-rust/blob/v0.20.0/crates/playwright/src/protocol/route.rs#L155)) and `Locator::and_()` and `or_()` (locator.rs L1083, L1140). That is Playwright Python's aliases, not `r#continue`. `and` and `or` are not even Rust keywords.

**B5. VERIFIED.**

- **serde.** In serde v1.0.229, `fn unraw` does `trim_start_matches("r#")` ([attr.rs L133-L134](https://github.com/serde-rs/serde/blob/v1.0.229/serde_derive/src/internals/attr.rs#L133-L134)). It is applied to container, variant and field names (L520, L896, L1039).
- **schemars.** schemars 1.2.3 takes names from `serde_derive_internals` 0.30.0 (schemars_derive/Cargo.toml L22). Its test struct declares `pub r#type` ([structs.rs L102](https://github.com/GREsau/schemars/blob/v1.2.3/schemars/tests/integration/structs.rs#L102)), and the snapshot property is `"type"`.

**B6. VERIFIED (nuance on pydantic).**

- **Clients.** All three show the sync/async pair:
  - psycopg 3.3.6 exports `Connection` and `AsyncConnection` (`psycopg/__init__.py` L62, L75).
  - httpx 0.28.1 exports `Client`, `AsyncClient`, `HTTPError`, `HTTPStatusError` and `URL`.
  - openai-python v3.31.0 exports `OpenAI`, `AsyncOpenAI`, `APIError` and `APIConnectionError` ([__init__.py](https://github.com/openai/openai-python/blob/v3.31.0/src/openai/__init__.py#L59-L84)).
- **Stainless pattern.** It holds across vendors.
  - anthropic-sdk-python v1.14.0 exports `Anthropic`, `AsyncAnthropic`, `APIError` and `APIConnectionError`.
  - openai-node v7.34.0 has `class APIError` and `APIConnectionError` ([core/error.ts L6, L112](https://github.com/openai/openai-node/blob/v7.34.0/src/core/error.ts#L6-L112)) and the option `baseURL` (client.ts L354).
  - anthropic-sdk-typescript sdk-v0.134.0 is the same (L6, L116; `baseURL` L669).
- **pydantic.** pydantic v2.14.1 exports `JsonValue`, `HttpUrl` and `AnyHttpUrl` in camel-case acronym style, but also `UUID4` and `IPvAnyAddress`. It is mixed rather than a clean exception ([__init__.py L220](https://github.com/pydantic/pydantic/blob/v2.14.1/pydantic/__init__.py#L220)).

**B7. VERIFIED.**

- **sqlx v0.9.0.** `pub trait Connection` has `close(self)` and `ping(&mut self)` ([connection.rs L16-L46](https://github.com/launchbadge/sqlx/blob/v0.9.0/sqlx-core/src/connection.rs#L16-L46)). It is re-exported at the root and in `sqlx::prelude`, a "Convenience re-export of common traits" (src/lib.rs L163-L167).
- **diesel v2.3.14.** `pub trait Connection` ([connection/mod.rs L222](https://github.com/diesel-rs/diesel/blob/v2.3.14/diesel/src/connection/mod.rs#L222)).
- **The scope rule.** rustc 1.93.0 reports E0599, "items from traits can only be used if the trait is in scope".
- **Waiters too.** The waiters in aws-sdk-s3 1.152.0 are methods of a `Waiters` trait, for example `wait_until_bucket_exists`, so the same import rule applies.

**B8. VERIFIED.**

- **@types/json-schema 7.0.15.** `export interface JSONSchema7` (index.d.ts L608; `JSONSchema4` L57).
- **Zod 4.6.7.** Exports `toJSONSchema` (v4/classic/external.d.ts L11).
- **type-fest 5.10.0.** Exports `JsonValue`, `JsonObject`, `JsonArray` and `JsonPrimitive` (source/json-value.d.ts L8-L31).
- **Google TypeScript style guide** ([tsguide.html](https://github.com/google/styleguide/blob/e4d272eb151adfb5f382501265172a99c6a19671/tsguide.html), commit `e4d272e`): "Treat abbreviations like acronyms in names as whole words": use `loadHttpUrl`, not `loadHTTPURL`. A platform name may require otherwise, as `XMLHttpRequest` does.

## C. Deprecation and experimental markers

**C1. PARTLY.** PEP 702 (Final, Python 3.13), [pep-0702.rst](https://github.com/python/peps/blob/c56c5c526b070b2be981e9a257347a048f8fcc35/peps/pep-0702.rst#L104-L110):

- **What it marks.** The decorator "can be used on a class, function or method". That includes TypedDict and NamedTuple *definitions*, single overloads, and the overload implementation (L104-L110).
- **Left out.**
  - Whole modules are omitted explicitly (L328-L333).
  - Module constants, object attributes and function parameters are left out too. A `Deprecated[type, message]` modifier was considered and dropped (L335-L341).
- **Parameters.** They can be deprecated indirectly, by deprecating an overload (L321-L322).
- **TypedDict items.** They are not named; they fall in the uncovered attribute case.

Correction: TypedDict definitions are covered, and parameters are reachable through overloads.

**C2. VERIFIED.** pyright 1.1.414: `reportDeprecated` is "none" in off, basic and standard modes and "error" in strict. Its default is "none". Source: [configuration.md L406](https://github.com/microsoft/pyright/blob/1.1.414/docs/configuration.md#L406).

**C3. VERIFIED.**

- **Added in mypy 1.14.** The changelog entry reads "Support for @deprecated Decorator (PEP 702)", enabled with `--enable-error-code deprecated`, and promises that it "will be enabled by default in a future mypy version" ([CHANGELOG L3067-L3079](https://github.com/python/mypy/blob/v2.4.0/CHANGELOG.md#L3067-L3079)).
- **Still off at v2.4.0.** `DEPRECATED ... default_enabled=False` ([errorcodes.py L332-L337](https://github.com/python/mypy/blob/v2.4.0/mypy/errorcodes.py#L332-L337)).
- **Not in `--strict`.** `--strict` only sets its fixed flag list, from `--disallow-any-generics` to `--extra-checks`, and enables no error code (main.py L1470-L1474).
- **Local check.** mypy 1.19.1 `--strict` alone is silent on a `@deprecated` call. Adding `--enable-error-code deprecated` reports it.

**C4. VERIFIED.**

- **tsc reports nothing.** Codes 6385 ("'{0}' is deprecated.") and 6387 have category `Suggestion` and `reportsDeprecated: true`. That holds in TypeScript 6.0.3 ([diagnosticMessages.json](https://github.com/microsoft/TypeScript/blob/v6.0.3/src/compiler/diagnosticMessages.json)) and in the 7.0.2 Go port ([diagnostics_generated.go L2955-L2959](https://github.com/microsoft/typescript-go/blob/typescript/v7.0.2/internal/diagnostics/diagnostics_generated.go#L2955-L2959)). `tsc` does not print suggestions: local `tsc --strict --noEmit` 5.9.3 on a call to a `@deprecated` function exits 0 with no output.
- **typescript-eslint.**
  - Its rule docs say: "TypeScript doesn't report type errors for deprecated code on its own."
  - `@typescript-eslint/no-deprecated` was added in 8.3.0 (2024-08-26); the changelog spells it "[no-deprecation] add rule".
  - The rule needs type information and is in the `strict` config ([no-deprecated.ts L36-L40](https://github.com/typescript-eslint/typescript-eslint/blob/v8.71.2/packages/eslint-plugin/src/rules/no-deprecated.ts#L36-L40)).
- **oxlint.**
  - `typescript/no-deprecated` is in the pedantic category and backed by tsgolint ([no_deprecated.rs L62-L64](https://github.com/oxc-project/oxc/blob/apps_v1.84.0/crates/oxc_linter/src/rules/typescript/no_deprecated.rs#L62-L64)).
  - It "requires type information" (type-aware mode, `--type-aware`) and was added in v1.26.0 ([oxc.rs rule page](https://oxc.rs/docs/guide/usage/linter/rules/typescript/no-deprecated.html)).

**C5. VERIFIED.** Rust Reference: `deprecated` "may be applied to any item, trait item, enum variant, struct field", plus external block items and macro definitions. It is not allowed on trait implementation items. On a module or impl, the children inherit it ([diagnostics.md](https://github.com/rust-lang/reference/blob/19330a51e1437f23a9379339902c735093c5b296/src/attributes/diagnostics.md)).

- **Default level.** The lint `DEPRECATED` is `Warn` ([builtin.rs L1669-L1671](https://github.com/rust-lang/rust/blob/7df81381d027710708209f36b98d11b1c88f8d2e/compiler/rustc_lint_defs/src/builtin.rs#L1669-L1671)).
- **Local check.** rustc 1.93.0 warned on a function, a method, a struct field, a unit struct, an enum variant and a constant. It also warned on a function inside a deprecated module, a trait and a trait method.

**C6. PARTLY.** Pydantic `Field(deprecated=...)` exists, marked "version-added | v2.7.0" ([fields.md, Deprecated fields](https://github.com/pydantic/pydantic/blob/v2.14.1/docs/concepts/fields.md)).

- **What the docs promise.** Only two effects: a runtime deprecation warning when the field is read, and the JSON Schema `deprecated` keyword. They say nothing about static type checkers.
- **Local check** (mypy 1.19.1 with `--enable-error-code deprecated --strict`, pydantic 2.12.4).
  - Reading a `Field(deprecated=...)` field, or an `Annotated[int, deprecated(...)]` field, raises a runtime `DeprecationWarning` but no mypy error.
  - A `@deprecated` function in the same file is flagged.

This matches C1: attributes are out of PEP 702's scope. Pyright was not tested.

**C7. VERIFIED (nuance).** In microsoft/tsdoc `StandardTags.ts` ([bfa9fe9](https://github.com/microsoft/tsdoc/blob/bfa9fe9211174e62aeec661206a1fb445d85e28a/tsdoc/src/details/StandardTags.ts#L105-L107)):

- **`@deprecated`** is a block tag of *Core* standardization (L105-L107).
- **`@alpha`, `@beta` and `@experimental`** are modifier tags of *Discretionary* standardization (L26, L46, L150).
- **`@experimental`** has the "Same semantics as `@beta`", for tools without an alpha stage (L141-L142).

## D. Surfaces

**D1. VERIFIED.** MCP 2025-11-25, tools.mdx, "Tool Names" (from SEP-986, PR #1603, per that revision's changelog). Names **SHOULD** follow these rules:

- **Length.** 1 to 128 characters.
- **Case.** Case-sensitive.
- **Characters.** Only `A-Z a-z 0-9 _ - .`, with no spaces or commas.
- **Uniqueness.** Unique within a server.
- **Examples.** `getUser`, `DATA_EXPORT_v2`, `admin.tools.list`.

These rules are SHOULD, not MUST. The 2026-07-28 revision, which the versioning page names the **current** one, has the same text.

**`Tool` fields:**

- **2025-11-25.** `name` and `title` (BaseMetadata), `icons`, `description`, `inputSchema`, `execution`, `outputSchema`, `annotations`, `_meta`.
- **2026-07-28.** The same fields minus `execution`.
- **`ToolAnnotations`.** `title`, `readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`.

No revision has a `deprecated` field on `Tool` or `ToolAnnotations`. 2026-07-28 deprecates protocol features through a registry (SEP-2577), not single tools. Sources: [tools.mdx 2025-11-25 L217-L228](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/ff465f00f097901a6d1b662d57646767057feed3/docs/specification/2025-11-25/server/tools.mdx#L217-L228), [schema.ts 2026-07-28](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/ff465f00f097901a6d1b662d57646767057feed3/schema/2026-07-28/schema.ts).

**D2. VERIFIED (facts established; limits differ).**

**OpenAI** ([openapi.json at 0fce13c](https://github.com/openai/openai-openapi/blob/0fce13ce46465f8876e4a968a73c7bca3aa8e335/openapi.json)):

- **Chat Completions.** `FunctionObject.name`: "Must be a-z, A-Z, 0-9, or contain underscores and dashes", maximum length 64. openai-python carries the same docstring (shared_params/function_definition.py L14-L18).
- **Responses API, top level.** `FunctionTool.name` states no limit.
- **Responses API, namespaced.** Functions inside a `namespace` tool (`FunctionToolParam`) must match `^[a-zA-Z0-9_-]+$`, 1 to 128 characters.
- **Tool-search outputs.** These allow a dot: `[a-zA-Z0-9_.-]`, 1 to 128.
- **Namespace field.** A Responses `function_call` item carries a separate `namespace` field next to `name`.
- **Guide.** The function-calling guide states no name limit.

**Anthropic.** `name` "Must match the regex `^[a-zA-Z0-9_-]{1,128}$`". The same page advises prefixing tool names with the service, as in `github_list_prs` ([define-tools](https://platform.claude.com/docs/en/agents-and-tools/tool-use/define-tools)).

**A dot is not allowed in ordinary function names by either API.**

**D3. VERIFIED.** Claude Code permissions docs: `mcp__puppeteer__puppeteer_navigate` matches the `puppeteer_navigate` tool of the `puppeteer` server. Allow globs need a literal `mcp__<server>__` prefix ([permissions](https://code.claude.com/docs/en/permissions)).

Other forms ([mcp](https://code.claude.com/docs/en/mcp)):

- **Plugin servers.** `mcp__plugin_<plugin-name>_<server-name>__<tool-name>`. Any character outside `A-Z a-z 0-9 _ -` becomes `_`.
- **claude.ai connectors.** `mcp__claude_ai_<server>__<tool>`.
- **`claude mcp` server names.** Letters, digits, hyphens and underscores only.

**D4. VERIFIED.**

- **openai-python v3.31.0 README.** "Making custom/undocumented requests" offers `client.get`, `client.post` and the other verbs (L908-L929), and the `.with_raw_response.` prefix (L852-L860) ([README](https://github.com/openai/openai-python/blob/v3.31.0/README.md#L908-L929)).
- **openai-node v7.34.0 README.** `client.post('/some/path', {...})` (L691-L706), plus `.asResponse()` and `.withResponse()` (L606-L628).
- **Azure Python.** For example, azure-keyvault-secrets has `send_request(request: HttpRequest, *, stream=False)`, which "Runs a network request using the client's existing pipeline" ([client_base.py L141-L147](https://github.com/Azure/azure-sdk-for-python/blob/5d04553268c3ef1c723bbf3455786cdad00d1583/sdk/keyvault/azure-keyvault-secrets/azure/keyvault/secrets/_shared/client_base.py#L141-L147)).
- **Puppeteer.** puppeteer-core v25.14.0 has `CDPSession.send(method, params?, options?)` ([CDPSession.ts L123-L127](https://github.com/puppeteer/puppeteer/blob/puppeteer-core-v25.14.0/packages/puppeteer-core/src/api/CDPSession.ts#L123-L127)).
- **Playwright.** Playwright's `CDPSession.send` has the same shape (types.d.ts L18416).

**D5. PARTLY.** Smithy 1.74.1 defines both traits:

- **`paginated`.** It applies to an operation or a service, and "Smithy clients SHOULD provide abstractions" that iterate automatically ([behavior-traits.rst](https://github.com/smithy-lang/smithy/blob/1.74.1/docs/source-2.0/spec/behavior-traits.rst)).
- **`smithy.waiters#waitable`.** Waiters "are a client-side abstraction used to poll a resource" ([waiters.rst](https://github.com/smithy-lang/smithy/blob/1.74.1/docs/source-2.0/additional-specs/waiters.rst)).

**Generated idioms:**

- **Rust.** aws-sdk-s3 1.152.0 has `ListObjectsV2FluentBuilder::into_paginator()` and `Waiters::wait_until_bucket_exists()`.
- **JavaScript.** @aws-sdk/client-s3 3.1152.0 has `paginateListObjectsV2(config, input)` and `waitUntilBucketExists`, next to the older `waitForBucketExists`.

**Correction: boto3 is not generated from these traits.** botocore loads paginator and waiter definitions at runtime from JSON data files; `s3/2006-03-01/paginators-1.json` defines `ListObjectsV2`. `client.get_paginator(operation_name)` and `get_waiter(name)` are generic methods ([botocore client.py L1214, L1320](https://github.com/boto/botocore/blob/1.43.114/botocore/client.py#L1214)).

**D6. VERIFIED.** `deprecated` (boolean, default false) is in the [2019-09](https://json-schema.org/draft/2019-09/meta/meta-data) and [2020-12](https://json-schema.org/draft/2020-12/meta/meta-data) meta-data vocabulary meta-schemas. It is absent from the draft-07 meta-schema.

- **It is an annotation.** It belongs to the meta-data annotation vocabulary.
- **Effect.** Applications "SHOULD refrain from usage of the declared property" ([2020-12 Validation §9.3](https://json-schema.org/draft/2020-12/json-schema-validation)).
- **Scope.** It can mark single array items or properties.

## Bearing on the decision

Playwright is a working model of one catalog serving several languages. It has one camelCase source with full-caps acronyms, mechanical per-language rules (Python: snake case, the property rule, a trailing `_` for keywords) and about 150 explicit aliases. playwright-python now adds a reasoned, self-pruning table of the remaining divergences. The catalog can copy that shape. Its names should record word boundaries rather than one cased string, because the languages disagree on acronyms (`HTTPServerError`, `Uuid`, `loadHttpUrl`) and real libraries mix styles. Keywords take a trailing `_` in Python and `r#` in Rust; serde and schemars strip `r#` on the wire. Deprecation reaches users unevenly: Rust warns by default, Python checkers do so only when configured and never for attributes or parameters, and `tsc` never does. Code markers therefore need runtime warnings and the JSON Schema `deprecated` keyword alongside them. For agents, MCP has no per-tool deprecation, and OpenAI and Anthropic reject dots in tool names. Claude Code also adds a `mcp__<server>__` prefix within a 64 to 128 character limit, so short `[A-Za-z0-9_-]` names are the portable form. Typed SDKs commonly keep a generic call as an escape hatch (`client.post`, `send_request`, `CDPSession.send`).
