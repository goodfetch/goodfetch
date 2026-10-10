# Single-tenant engine; the cloud is an ordinary protocol client

Automan Cloud must power an actor platform, a crawl and extraction API, and hosted browsers, yet the engine has no notion of tenants or prices: each run gets its own engine, and the cloud isolates runs (container or VM) and turns usage into prices, as Apify, Spider Cloud and Browserbase do with their open-source libraries. The cloud drives the engine through the same JSON-RPC protocol as the SDKs, where usage, limits and lifecycle signals (checkpoint, abort, migrate, resume) are ordinary messages, so one conformance suite covers both and anything the cloud does can be reproduced locally. Decided in [Cloud readiness: which cloud businesses the engine must serve](https://github.com/Jbmanllr/automan/issues/69).

## Considered Options

- **Multi-tenant engine** (one process serves many tenants): rejected. One crash or hang in it would stop every tenant's runs, and the lab showed that a pool reused without reset leaks one tenant's cookie to the next 30 times out of 30.
- **Separate worker protocol for the cloud**: rejected. Two protocols means two conformance suites, and cloud behaviour that cannot be reproduced locally.

## Consequences

- A reset or discard must leave no state behind; a permanent conformance gate checks it.
- The engine counts usage and enforces limits but never knows prices.
