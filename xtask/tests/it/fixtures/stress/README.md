# Stress fixture

A workspace whose tests `xtask stress select` chooses from: in each runner,
a file the change edits (`changed`), a file it leaves alone (`steady`) and,
for Vitest and Playwright, a file whose import the change edits (`imports`).
`quarantine.toml` lists each runner's `flaky` test. Each test commits this as
the base, then makes its change.

It is a pnpm workspace and a uv workspace, as the repository is. Its runners
are the repository's installed ones, so it installs offline: Vitest and
Playwright through `.toolchain`, a link each test makes and commits; pytest
through the member `.pytoolchain`, whose `.pth` file and the interpreter in
`.python-version` each test writes and commits. The Vitest config imports the
workspace package `web-support`, and `tests/conftest.py` the member
`py-support`, so a base listed with the head's packages fails to load them.
