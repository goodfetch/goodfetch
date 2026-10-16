# A failure from a party Goodfetch did not write becomes a typed error in one module per boundary, and no other code reads its text

Every module that talks to a party Goodfetch did not write (the browser over CDP, a site over a fetch tier, a proxy, a Model provider, a Sink's service) turns that party's failures into typed variants in one place, its **translation point**. The party's own text stays in a private field: it reaches logs and diagnostics but is never offered as a string to compare, so no other code can match on it. An unknown failure becomes a catch-all variant that keeps the text, maps to the generic code the area spec names, and is logged at `warn`. Each translation point has a test table with one named case per known message or code.

The outbound side was decided: one Error code per remedy, a retryable flag and a class per code generated into every SDK (ADRs 0038 and 0079), and tests assert a failure on its code, never its text (`TESTING.md`). The inbound side was decided case by case: a proxy's own 429 or 502 is a proxy error, never a Block cause (ADR 0053); Model provider errors are classified by type and message (ADR 0101); a page closed under a call (ADR 0113); a `cdp_url` the engine cannot use (ADR 0115); transport failures on the fetch tiers (ADR 0118). No rule said where that classification happens, so each boundary could match text wherever it needed. The [competitor codebase design research](https://github.com/goodfetch/goodfetch/blob/research/competitor-codebase-design/rnd/research/reports/Competitor%20codebase%20design.md) ([#555](https://github.com/goodfetch/goodfetch/pull/555)) found that is what fails:
- **Matching on text wherever it is needed.** Crawlee JS detects proxy errors by substring. Stagehand recognizes CDP errors by message ("Cannot find context with specified id"). Browser Use classifies by substring, flattens errors into `RuntimeError` strings, and has 526 broad `except Exception` handlers, 46 of them a silent `pass`. A browser or library that rewords a message breaks each of those sites separately.
- **The fix, in part.** Scrapy's download handlers (HTTP/1.1, HTTP/2, aiohttp, httpx, FTP) all raise one transport-neutral family (`DownloadTimeoutError`, `CannotResolveHostError` and the rest). Playwright turns every "target closed" protocol error into one `TargetClosedError` carrying the close reason.
- The error model ties with duplication as the field's lowest-scored dimension, a mean of 2.1 of 5.

Decided in [A failure from CDP, a fetch, a proxy or a model has no single place where it becomes a typed error](https://github.com/goodfetch/goodfetch/issues/571) ([grilling record](https://github.com/goodfetch/goodfetch/issues/571#issuecomment-6214830281)). The owner decided the rules; the agent decided their details on the owner's delegation of 2026-10-16. One detail departs from the grilling record, which named one translation point for the fetch layer every tier shares and one for the proxy layer: each fetch tier reaches a different library, and a proxy's refusal arrives through whichever library or browser met it, so the text is read where each failure arrives, and what the tiers share is the enum and its handling (ADR 0129). No SDK API changes, so ADR 0077's principles do not apply.

## Decision

### One translation point per boundary

- **The translation points are:**
  - the CDP client, for every CDP error and every disconnection of the browser;
  - the transport primitive of each fetch tier that sends HTTP itself (ADR 0129), for its library's transport, TLS and HTTP failures; a browser tier's failures reach the CDP client;
  - each Model provider adapter (ADRs 0012 and 0101), where ADR 0101's classification by error type and message happens;
  - each built-in Sink's client (ADR 0026);
  - Collie's TLS relay (ADR 0124), for the TLS failures it meets on a site;
  - any later module that talks to such a party, which its area spec names.
- **A proxy's refusal is classified only where an engine module sees the proxy's own answer.** That is the transport primitive of a tier that sends HTTP itself, which reads the answer to its tunnel request, and ADR 0017's egress proxy on a path where it forwards to an upstream proxy, which is then that path's translation point. There ADR 0053's proxy refusals get their own variants. A browser reached without the egress proxy shows less: Chromium reports any refused tunnel but a 407 as one network error (`ERR_TUNNEL_CONNECTION_FAILED`), and a 407 as `Fetch.authRequired` once the CDP client has turned on the Fetch domain with `handleAuthRequests`. The CDP client maps those, and area 5's spec decides how such a failure counts under ADR 0053. Through a proxy to a plain `http://` URL there is no tunnel, so a proxy's own 429 or 502 arrives as an ordinary response on every tier; area 5's spec decides how it is told from the site's.
- **Process Plugins and SDK handlers need none.** They return typed errors through the plugin protocol (ADR 0033), whose engine side checks them like any other decoded input.
- **A translation point is the module that first receives the party's failure.** Where one implementation talks to the party, it owns its error enum of typed variants, written with `thiserror` (`STYLE.md`), from which its crate maps to Error codes. Only the RPC layer converts to the wire, as before.
- **Where a driver sits above several implementations (ADR 0129), the driver defines the enum once** and decides what each variant leads to (a retry, a backoff, an Error code). Each implementation's translation point maps its party's failures into that enum, since each reaches a different library or format. The driver never reads a party's text.
- **Matching on a party's text happens only inside the translation point**, against literals kept beside the enum. Everywhere else, code matches on variants.

### The text is kept, not exposed

- **The party's own message is a private field** of a newtype with a `Display` and a `Debug` that redact Secrets and credentials as every log does (ADRs 0014 and 0015, tested under ADR 0067). It has no accessor that returns `&str` or `String`.
- **Formatting an error and searching the result is refused in review.** Hiding the field makes the wrong path a detour that review sees; no lint can tell such a search from formatting for a log.

### Unknown failures and tests

- **A failure the translation point does not recognize becomes its `Unrecognized` variant.** It keeps the text for diagnosis, maps to the generic code the area spec names for that operation (such as `navigation_failed`, ADR 0085), and is logged at `warn` with the text, so a new message is noticed and added to the table.
- **Each translation point has a test table:** one named case per known message or code (rstest's `#[case::name]`, `TESTING.md`), each with the variant it must become, plus a case for an unknown failure. Where a recording of the party exists (a Model recording, ADR 0065; a Response recording, ADR 0105), its failures join the table.

## Considered Options

- **Classify wherever the failure is handled:** rejected. It is what Crawlee, Stagehand and Browser Use do, and a reworded message then breaks each site separately.
- **An `xtask` check that greps for string matching on errors:** rejected. It is noisy and easy to dodge, and it cannot tell a log line from a comparison.
- **Review alone, with the text public:** rejected. It leaves the easy path open.
- **Drop the text after classifying:** rejected. An unknown failure would carry nothing to diagnose it with, and an Error code carries what was observed where that applies (ADR 0079's fields).
- **An unknown failure as an `InternalError`:** rejected. A party sending a new message is not a bug in Goodfetch; the operation's own code names the remedy.

## Consequences

- **`STYLE.md`.** Errors states the translation-point rule and the private text.
- **`TESTING.md`.** The review checklist checks that a new upstream message or code has its case in its translation point's table.
- **ADR 0053 and ADR 0101** carry notes: the proxy refusal and the Model provider classification each happen in their translation point.
- **ADR 0129** states the drivers whose enums the translation points of the fetch tiers, the Model provider adapters and the Sinks' clients map into.
- **ADR 0037.** Every area follows this ADR, as it follows ADR 0126, since any area may hold a module that talks to such a party: the egress proxy and Secret providers (area 4), Notification channels (area 11) and the rest. Areas 3, 5, 6 and 9 also list it in their rows. Each spec names its translation points and the generic code an unknown failure maps to.
