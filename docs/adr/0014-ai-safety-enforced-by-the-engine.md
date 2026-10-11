# AI safety is enforced by the engine, not by the model or the client

The engine enforces each run's domain policy on every request type (navigation, subresources, workers, WebSockets, WebRTC, `javascript:` URLs) through browser-protocol interception, and fails closed: a request the policy cannot judge is blocked. Page content is wrapped as untrusted in every observation. Effects an AI decides in the categories payment, account creation, accepting terms, sending messages and deletion ask for confirmation unless the run allows them, and vendor safety signals are journaled. Secrets are `%var%` placeholders the engine types at the moment of input, bound to the domains allowed to receive them, with the target field re-checked first; they never reach the model, Plans, caches, the journal, observations or logs, and are masked in screenshots, recordings and traces. Decided in [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72).

## Considered Options

- **Allowlists in the client library**: rejected. Domain allowlists were bypassed in Stagehand, Browser Use and Skyvern, and Playwright MCP calls its own filters "not a security boundary".
- **Relying on the model vendors' safety**: rejected as the only layer. OpenAI, Anthropic and Google ship classifiers and confirmations, but local and gateway models may not.

## Consequences

- An injection classifier on pages can be switched on (Firecrawl's precedent).
- Secrets are stored behind Secret references and Secret providers (ADR 0015). A run may switch redaction off, which lifts the guarantee above for logs, traces, HAR files, screenshots and recordings kept as outputs only (ADR 0016).
- A raw CDP client can switch off browser-protocol interception, so a Session with raw CDP attached is held to the domain policy at the network layer instead (ADR 0017).
- For payments, a run's allowance is `pay_domains`, which only Operator policy, user and project configuration or a person's answer can widen; no payment goes to a domain outside it (ADR 0045).
