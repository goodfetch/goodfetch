# Single-tenant engine; the cloud is an ordinary protocol client

Goodfetch Cloud must power an actor platform, a crawl and extraction API, and hosted browsers, yet the engine has no notion of tenants or prices: in Goodfetch Cloud each run gets its own engine (a local Daemon hosts many runs, ADR 0024), and the cloud isolates runs (container or VM) and turns usage into prices, as Apify, Spider Cloud and Browserbase do with their open-source libraries. The cloud drives the engine through the same JSON-RPC protocol as the SDKs, where usage, limits and lifecycle signals (checkpoint, abort, migrate, resume) are ordinary messages, so one conformance suite covers both and anything the cloud does can be reproduced locally. Decided in [Cloud readiness: which cloud businesses the engine must serve](https://github.com/Jbmanllr/automan/issues/69).

## Considered Options

- **Multi-tenant engine** (one process serves many tenants): rejected. One crash or hang in it would stop every tenant's runs, and the lab showed that a pool reused without reset leaks one tenant's cookie to the next 30 times out of 30.
- **Separate worker protocol for the cloud**: rejected. Two protocols means two conformance suites, and cloud behaviour that cannot be reproduced locally.

## Consequences

- A reset or discard must leave no state behind; a permanent conformance gate checks it. Saved identities are the one exception: named objects that outlive runs by design, kept outside run state and removed only by their own delete (ADR 0035). A Profile directory a run uses in place is the person's own data, not Goodfetch state, so the gate neither removes nor scans it (ADR 0056).
- The engine counts usage and enforces limits but never knows prices.
- Goodfetch Cloud chooses the version of the Clean-up rules for each run and hands its digest to the engine through the protocol; a local run reproduces it with `clean_up.version` (ADR 0047).
