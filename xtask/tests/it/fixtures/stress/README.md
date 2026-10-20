# Stress fixture

A workspace whose tests `xtask stress select` chooses from: in each runner,
a file the change edits (`changed`), a file it leaves alone (`steady`) and,
for Vitest and Playwright, a file whose import the change edits (`imports`).
`quarantine.toml` lists each runner's `flaky` test. Each test commits this as
the base, then makes its change.

It is a pnpm workspace, as the repository is. Its runners are the
repository's installed packages, reached through `.toolchain`, a link each
test makes and commits. The Vitest config imports the workspace package
`web-support`, so a base listed with the head's packages fails to load it.
