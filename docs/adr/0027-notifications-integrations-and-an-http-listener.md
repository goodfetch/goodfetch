# Notifications and Integrations are generated from the Operation catalog and reach the engine over an HTTP listener

Messages about a run or a Schedule (Notifications; ADR 0036) are kept apart from Records (Sinks). Goodfetch inside other tools (Integrations) is a Client generated from the Operation catalog, like every agent surface in ADR 0011.

Workflow tools force one change to ADR 0024. n8n's verification rules forbid runtime dependencies and environment or file access, so a verified node cannot start the engine and, we infer, can only reach it over HTTP; Zapier, Make and Pipedream call from their own clouds. So the Daemon gains an **HTTP JSON-RPC listener**: the same protocol with the same `hello` version check. It is off until a setting enables it, then binds to loopback with a token, and needs TLS plus a token for remote use (ADR 0017). In a Daemon group, the Lead Daemon holds that token, so an Integration may call any Group member (ADR 0055).

> Amended on 2026-11-11 by the owner (grilling of decision G-19 of the consolidated review, Q10: « Yes to all », on option (a) as put; the owner did not read this note's wording): the Daemon also answers Firecrawl's and Jina Reader's APIs, each behind its own setting, off by default. As put: each Firecrawl request is translated into Goodfetch's operations (scrape, batch scrape, crawl, map, search, extract); a route whose capability Goodfetch lacks answers "not supported" in Firecrawl's error shape; Jina Reader's route answers Markdown; no code is copied. Before answering, the owner was shown ADR 0037:64's reason against a compatibility layer, which this reverses for these routes (ADR 0037's note of that day), and was told that the listener above speaks only Goodfetch's JSON-RPC protocol, so these routes are the first other protocol shape on it. Their rows are CL-10 and CL-11 of ADR 0037's matrix.
>
> Note (the grilling session's own, 2026-11-11, same grilling, awaiting the owner's reading): the clients of these routes send no `hello`. A Firecrawl client authenticates with `Authorization: Bearer <token>`, which the routes check against the listener's token, and the routes bind, need TLS for remote use and are refused as the listener is (ADR 0017). "map" is a new operation (CO-73), and the Firecrawl route waits for it. Jina Reader's own form is `r.jina.ai/<url>` (from Jina's public usage, not checked on the web); the route serves `GET /<prefix>/<url>` under a prefix its setting names. `/r/` is pagesift's prefix (`crates/pagesift-api/src/jina/mod.rs:20#jina_routes`, pagesift v1.2.4).

Apify, Firecrawl, Bright Data and ScrapingBee all ship verified n8n nodes, Make apps and Zapier apps. Users' most frequent complaint is a run that looks green but delivered nothing, which is what Notifications and per-page events address. Decided in [Export connectors and integrations catalog](https://github.com/goodfetch/goodfetch/issues/88).

Evidence: [workflow tools, Airbyte and AI loaders](https://github.com/goodfetch/goodfetch/blob/research/sinks-integrations-catalog/rnd/research/2026-10-11-sinks-integrations/b1-integrations.md) and [notification channels](https://github.com/goodfetch/goodfetch/blob/research/sinks-integrations-catalog/rnd/research/2026-10-11-sinks-integrations/a3-notify-vendors.md).

## Considered Options

- **Hand-written Integrations per tool**: rejected for the same reason as ADR 0011. They drift from the engine, while generated ones expose every operation and setting the same way.
- **Workflow tools reaching the engine only through Goodfetch Cloud**: rejected. Self-hosted n8n and Activepieces can reach a local Daemon directly, and Goodfetch Cloud is a later map.
- **Notifications as ordinary webhook Sinks**: rejected. A Notification carries no Records, goes to people as well as systems, and must not repeat across a resumed run. Most channels have no idempotency key: only Resend and Discord's bot API have one.

## Consequences

**Notification channels**:
- Slack
- Microsoft Teams, through Workflows webhooks only, since Microsoft switched off Office 365 connectors in May 2026
- Discord
- Telegram
- email: SMTP, SES, Resend, Postmark, SendGrid and Mailgun
- Standard Webhooks

**Notification events**:
- run started, finished, failed, or stopped by a Limit
- Completeness `incomplete`
- a Sink `behind`, `failed`, paused or recovered
- a schema change
- `auth expired`
- an Effect reported uncertain
- an approval requested
- a Human pause waiting, with a link to the live view (ADR 0006)
- records changed against the run's Baseline (ADR 0036)
- a Schedule auto-paused, its occurrences skipped by a Limit (once per period), or its occurrences missed (once per wake) (ADR 0036)
- a Saved identity's earliest cookie expiry entering its warning window (ADR 0056)

**Notification delivery**: an engine-side ledger keyed by event ID means a resumed run never re-sends. Channel idempotency keys are used where they exist. A status change notifies once, not once per run.

**Integrations in V1**:
- n8n (a verified node), Zapier, Make, Pipedream, Activepieces and Power Automate. Their actions start runs and Recipes and fetch Records. Their triggers arrive through a thin webhook carrying a record range and a fetch URL.
- An Airbyte source that reads Datasets incrementally by record number. Apify's Airbyte source claims incremental sync but its manifest has none.
- Dataset loaders for LangChain and LlamaIndex in TypeScript and Python, published as Goodfetch's own packages. LlamaIndex's shared web-readers package is now GPL-3.0.

  > Amended by [ADR 0070](0070-mcp-adapters-and-integrations-are-tested-through-their-platforms-own-clients-against-a-real-engine.md): the LangChain Dataset loader ships in TypeScript and Python inside the LangChain package that carries the Framework adapter; LlamaIndex gets a Python reader only, since LlamaIndex.TS is archived.

  > Amended by [ADR 0095](0095-both-products-speak-one-protocol-from-one-catalog-marked-by-product-and-their-sdks-share-collies-client-packages.md): each workflow tool gets one Integration per Product. Collie's start runs and Recipes and return their results, with Notifications as triggers; Foxhound's add crawls, Records and the record-range triggers. The Airbyte source and the Dataset loaders read Datasets, so they are Foxhound's only.

**What each Integration can reach**: the docs for each Integration state whether it works with a local Daemon, an exposed Daemon, a tunnel or Goodfetch Cloud. Zapier rejects localhost; Make reaches local services only through its Enterprise on-premise agent; Power Automate's on-premises data gateway supports neither API-key authentication nor custom code.

**The HTTP listener** also serves the thin webhook's fetch URL. Being on HTTP, it keeps ADR 0017's `Host` and `Origin` checks. The Console does not read runs through it: it speaks the protocol over its own WebSocket (ADR 0031).
