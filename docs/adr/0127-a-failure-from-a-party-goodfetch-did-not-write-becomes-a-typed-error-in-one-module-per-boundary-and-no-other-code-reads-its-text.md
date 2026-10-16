# A failure from a party Goodfetch did not write becomes a typed error in one module per boundary, and no other code reads its text

Every module that talks to a party Goodfetch did not write (the browser over CDP, a site over a fetch tier, a proxy, a Model provider, a Sink's service) turns that party's failures into typed variants in one place, its **translation point**. The party's own text stays in a private field: it reaches logs and diagnostics but is never offered as a string to compare, so no other code can match on it. An unknown failure becomes a catch-all variant that keeps the text, maps to the generic code the area spec names, and is logged at `warn`. Each translation point has a test table with one named case per known message or code.

The outbound side was decided: one Error code per remedy, a retryable flag and a class per code generated into every SDK (ADRs 0038 and 0079), and tests assert a failure on its code, never its text (`TESTING.md`). The inbound side was decided case by case: a proxy's own 429 or 502 is a proxy error, never a Block cause (ADR 0053); Model provider errors are classified by type and message (ADR 0101); a page closed under a call (ADR 0113); a `cdp_url` the engine cannot use (ADR 0115); transport failures on the fetch tiers (ADR 0118). No rule said where that classification happens, so each boundary could match text wherever it needed. The [competitor codebase design research](https://github.com/goodfetch/goodfetch/blob/research/competitor-codebase-design/rnd/research/reports/Competitor%20codebase%20design.md) ([#555](https://github.com/goodfetch/goodfetch/pull/555)) found that is what fails:
- **Matching on text wherever it is needed.** Crawlee JS detects proxy errors by substring. Stagehand recognizes CDP errors by message ("Cannot find context with specified id"). Browser Use classifies by substring, flattens errors into `RuntimeError` strings, and has 526 broad `except Exception` handlers, 46 of them a silent `pass`. A browser or library that rewords a message breaks each of those sites separately.
- **The fix, in part.** Scrapy's download handlers (HTTP/1.1, HTTP/2, aiohttp, httpx, FTP) all raise one transport-neutral family (`DownloadTimeoutError`, `CannotResolveHostError` and the rest). Playwright turns every "target closed" protocol error into one `TargetClosedError` carrying the close reason.
- The error model ties with duplication as the field's lowest-scored dimension, a mean of 2.1 of 5.

Decided in [A failure from CDP, a fetch, a proxy or a model has no single place where it becomes a typed error](https://github.com/goodfetch/goodfetch/issues/571) ([grilling record](https://github.com/goodfetch/goodfetch/issues/571#issuecomment-6214830281)). The owner decided the rules; the agent decided their details on the owner's delegation of 2026-10-16. No SDK API changes, so ADR 0077's principles do not apply.

## Decision

### One translation point per boundary

- **The translation points are:**
  - the CDP client, for every CDP error and every disconnection of the browser;
  - the fetch layer every fetch tier shares (ADR 0129), for transport, TLS and HTTP failures, with what differs per tier in each tier's primitives;
  - the proxy layer, where ADR 0053's proxy refusals are told apart from a site's answer;
  - each Model provider adapter (ADRs 0012 and 0101), where ADR 0101's classification by error type and message happens;
  - each built-in Sink's client (ADR 0026);
  - any later module that talks to such a party, which its area spec names.
- **Process Plugins need none.** They return typed errors through the plugin protocol (ADR 0033), whose engine side checks them like any other decoded input.
- **Each translation point owns one error enum** of typed variants, written with `thiserror` (`STYLE.md`), from which its crate maps to Error codes. Only the RPC layer converts to the wire, as before.
- **Matching on a party's text happens only inside the translation point**, against literals kept beside the enum. Everywhere else, code matches on variants.

### The text is kept, not exposed

- **The party's own message is a private field** of a newtype with a `Display` and a `Debug` that go through ADR 0015's redaction. It has no accessor that returns `&str` or `String`.
- **Formatting an error and searching the result is refused in review.** Hiding the field makes the wrong path a detour that review sees; no lint can tell such a search from formatting for a log.

### Unknown failures and tests

- **A failure the translation point does not recognize becomes its `Unrecognized` variant.** It keeps the text for diagnosis, maps to the generic code the area spec names for that operation (such as `navigation_failed`, ADR 0085), and is logged at `warn` with the text, so a new message is noticed and added to the table.
- **Each translation point has a test table:** one named case per known message or code (rstest's `#[case::name]`, `TESTING.md`), each with the variant it must become, plus a case for an unknown failure. Where a recording of the party exists (a Model recording, ADR 0065; a Response recording, ADR 0105), its failures join the table.

## Considered Options

- **Classify wherever the failure is handled:** rejected. It is what Crawlee, Stagehand and Browser Use do, and a reworded message then breaks each site separately.
- **An `xtask` check that greps for string matching on errors:** rejected. It is noisy and easy to dodge, and it cannot tell a log line from a comparison.
- **Review alone, with the text public:** rejected. It leaves the easy path open.
- **Drop the text after classifying:** rejected. An unknown failure would carry nothing to diagnose it with, and ADR 0077's G6 requires that a failure says what was observed.
- **An unknown failure as an `InternalError`:** rejected. A party sending a new message is not a bug in Goodfetch; the operation's own code names the remedy.

## Consequences

- **`STYLE.md`.** Errors states the translation-point rule and the private text.
- **`TESTING.md`.** The review checklist checks that a new upstream message or code has its case in its translation point's table.
- **ADR 0053 and ADR 0101** carry notes: the proxy refusal and the Model provider classification each happen in their translation point.
- **ADR 0037.** Areas 3 (Engine runtime), 5 (Site access), 6 (Collection) and 9 (AI and agent) list this ADR. Each spec names its translation points and the generic code an unknown failure maps to.
