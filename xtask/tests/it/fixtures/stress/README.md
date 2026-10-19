# Stress fixture

A workspace whose tests `xtask stress select` chooses from: in each runner,
a file the change edits (`changed`), a file it leaves alone (`steady`) and,
for Vitest and Playwright, a file whose import the change edits (`imports`).
`quarantine.toml` lists each runner's `flaky` test. Each test commits this as
the base, then makes its change.
