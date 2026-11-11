# Goodfetch

The repository of Goodfetch's two Products: Collie, for browser automation, and Foxhound, for crawling, which contains Collie ([ADR 0084](docs/adr/0084-goodfetch-ships-two-products-collie-and-foxhound-from-one-repository-and-foxhound-contains-collie.md)).

Collie drives Chromium-family browsers only for now (Chrome, Chromium, and Edge on a best-effort basis). Firefox and WebKit are planned and not built yet.

The [`rnd/`](rnd/) folder holds R&D material kept for inspiration only: the original proposals, the plan, the proposed ADRs, the sourced research, the [lab](rnd/lab/README.md) that measured each choice, and the [R&D report](rnd/README.md), which compares the stack proposed before the tests with the stack recommended after them, along with the evidence and the questions still open. Nothing in it is an adopted decision.

`docs/` is reserved for the Products' final documentation. `docs/agents/` configures the engineering skills.

The repository has been a dedicated Git repository since October 8, 2026. Campaign outputs (`rnd/lab/results/`), dependencies, browsers, toolchains and native builds are not versioned; the retained evidence remains tracked in `rnd/lab/ledger/` and `rnd/lab/reports/`.
