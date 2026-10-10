# Language and architecture: consensus or observed convergences?

Survey of **October 7, 2026**. This note reuses the dossier's inspections and directly checks four families of primary sources: Stagehand, Crawlee/Impit, Spider and agent-browser. No new benchmark, community poll or Reddit sweep was carried out. Readings of `main` describe the current sources, not necessarily a distributed release. The hypothesis of a small team and a limited timeline has been removed.

## What can be asserted

**The sources examined establish no consensus on a best language.** They show several viable architectures and distinct decisions depending on the product layer. The presence of Rust in several projects establishes its use, not a majority preference of the community, a general superiority or an obligation to migrate. We have neither a representative survey nor a comparison covering the same contracts, resources and results.

**Stagehand v4** moves the management of targets, frames and CDP commands into an extension close to the page. The TypeScript, Python and Go SDKs communicate with this shared runtime. The bidirectional protocol is based on JSON-RPC; its Zod schemas produce JSON Schema and client models. The documented decision concerns the placement of state and the unity of contracts, without imposing Rust. The blog's batching figure is explicitly a single flow, not a general ranking. [v4 architecture](https://www.browserbase.com/blog/stagehand-v4), [current protocol](https://github.com/browserbase/stagehand/blob/main/packages/protocol/README.md).

**Crawlee TypeScript** separates HTTP/browser acquisition from the queue, storage, sessions, hooks and concurrency regulation mechanisms. Its Impit adapter allows a native transport: Impit is a Rust library with Node and Python interfaces. This is a concrete counterexample to the opposition "TypeScript SDK, therefore the whole costly path is JavaScript". The 3.18 documentation announces Impit as the future default of the next major version; this does not mean it is already the default everywhere. [Crawlee introduction](https://crawlee.dev/js/docs/introduction), [Impit adapter](https://crawlee.dev/js/docs/guides/impit-http-client), [Impit library](https://github.com/apify/impit).

**Spider** uses Rust, Tokio and HTTP transports; its current manifest links the Chrome path to Chromey. Features determine the available capabilities. This choice provides a serious native reference, but the manifest proves neither actionability equivalent to Playwright nor a correct business result. The detailed findings on release 2.53.9 remain separate from the changes in `main`. [Current manifest](https://github.com/spider-rs/spider/blob/main/spider/Cargo.toml), [release inspection](2026-10-07-crawlee-spider-contracts-followup.md).

**agent-browser** currently documents a Rust CLI and daemon using CDP directly, with no Node or Playwright needed by the daemon. The process persists between commands. This confirms the feasibility of a native executor with sessions; it does not demonstrate that reproducing its driver would be preferable to reusing Playwright for every need. [Current architecture and requirements](https://github.com/vercel-labs/agent-browser#architecture).

## Convergences more solid than the ranking of languages

**Inference from these architectures:** distinguishing developer interface, execution and transport seems more useful than choosing a single language for all layers. An SDK can remain idiomatic while the engine, the browser and the transport live elsewhere. Proximity to the browser, reused sessions, grouped commands and HTTP when sufficient can reduce work and exchanges independently of the language.

These convergences remain partial. The Stagehand extension and the native daemon do not have the same lifecycle; the Crawlee crawler and an agent do not have the same recovery rules. No shared protocol makes their `Page` objects interchangeable. Schemas guarantee a message shape; functional parity also requires behavior tests.

## Advantages and costs for Automan

The following table is a **design analysis**, not a community verdict.

| Choice | Advantage sought | Cost to accept |
| --- | --- | --- |
| TypeScript | Direct integration with the Playwright/Crawlee ecosystem, recipes and developer tools; native transport possible without rewriting the whole engine. | Node deployment and dependencies; multiple clients require a protocol or bindings; CPU operations and allocations must be profiled. |
| Rust | Common library and executable, resource control and sharing of behavior across SDKs; consistent candidate for HTTP, streaming and workers. | Per-platform packaging, async complexity, IPC and driver qualification. Memory guarantees prove neither data freshness nor the success of a workflow. |
| Python | SDK, extraction extensions and integration with applications/models; possibility of delegating operations to the native engine. | Environment and distribution to manage; runtime duplication to avoid. Network and browser waits make it impossible to infer latency from the language alone. |
| Go | Plausible alternative for a concurrent service and a standalone executable, or as a client of the shared protocol. | Evaluate browser libraries and transports according to the required capabilities; a second Go engine would create a second implementation to maintain. |

## Contextualized recommendation

For the announced ambition, **canonical Rust with TypeScript/Python SDKs** is a consistent recommendation; Go can become an additional client. This choice targets distribution, unity of behavior and resource control. It does not result from a demonstrated consensus.

The engine should separate crawl frontier, durable plans, sessions, HTTP/browser acquisition, semantic resolution and validation. A native CDP backend remains a candidate to be qualified; Playwright provides a reference and a compatibility backend. The versioned protocol must make capabilities, errors, cancellation, budgets and ownership explicit. Routing chooses the path suited to the expected result, without imposing an HTTP attempt before each browser.

The decision becomes verifiable with a common corpus: fresh and complete output, wrong target still clickable, CDP loss after an effect, auth expiry and sink interruption. Publish full cost, valid throughput, total memory, false successes and interventions. **Rust can improve a component; only the system benchmark will prove Automan's advantage.**
