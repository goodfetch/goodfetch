# AI success is never judged by a model alone

An AI task succeeds only when a declared Success check passes, evaluated live by the engine; without one, the result is `needs-review`, and a model's "done" or an LLM judge is a signal, never proof. The built-in agent is a thin journaled loop over the same Operation catalog an outside agent uses, so any outside agent can replace it; it serves login recovery, Repair, extraction discovery and the user's objective tasks. AI extraction applies the same rule through Grounding: every extracted value must appear in the page or follow from a declared transform, and in compiled mode values are copied from the selected elements, never written by the model. Decided in [Adopt AI and agent decisions](https://github.com/Jbmanllr/automan/issues/72).

## Considered Options

- **An LLM judge by default** (Browser Use): rejected. On the only independent benchmark, Online-Mind2Web, Browser Use succeeded on 30% of tasks against the 89% it reports on WebVoyager.
- **No built-in agent** (Stagehand v4 removed `agent()`, Cua deprecated its own): rejected. Login recovery, Repair and extraction discovery need one; keeping it thin over the catalog avoids the embedded loops that went dormant (LaVague, Bytebot, Agent-E).
- **Per-page model extraction as the default** (Firecrawl, Crawlee's `PydanticAiCrawler`): rejected; selectors written once by the model and replayed deterministically are the default, per-page extraction an explicit mode.

## Consequences

- Vision is opt-in and the last rung of the ladder; a coordinate action carries its screenshot's id and is refused if the page changed since.
- Compiled selectors are keyed by route and page layout and repaired when validation failures appear; an LLM clean-up of Markdown is an opt-in step.
