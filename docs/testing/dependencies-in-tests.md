## Dependencies in tests

A module is tested according to the kind of dependency it has, using the four categories of `codebase-design`:

- **In-process** (pure logic: URL normalization, robots, frontier rules, extraction, run-state rules): through the module's interface; `proptest` for parsers.
- **Local-substitutable** (a real local stand-in exists): SQLite in a temporary file; a real Chromium (the Goodfetch build of ADR 0035) against Proving ground pages served locally. CDP is never mocked.
  _Why:_ a fake browser lets tests pass where a real one fails, which is the false success ADR 0025 measures.
- **Remote but owned** (Goodfetch's own processes across a boundary: sink processes, process Plugins, the Daemon): the logic is tested through an in-memory adapter of the protocol and, for several hosts, in the [Simulation](simulation-model-checking-and-crash-testing.md#simulation), and the real processes through the conformance suites.
- **True external** (model providers, CAPTCHA services): a stand-in adapter. For Model providers, the Scripted model in-process, Model recordings replayed by `httpmock` at the adapter's base URL (matched on the body, secrets removed by an allowlist), and the Proving ground's Fake model end to end; no test calls a live model ([AI features and agent skills](ai-features-and-agent-skills.md), ADR 0065).
- **An `httpmock` test binds a socket, so it sits in `tests/it`**, never among the unit tests ([The three suites](suites-commands-and-time-budgets.md#the-three-suites)).

How each dependency is injected is in [Injection and determinism](injection-and-determinism.md).
