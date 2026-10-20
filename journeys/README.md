# Journeys

The Journey catalog (ADR 0063): one Journey per file, `J-NNN.toml`, and
`catalog.toml`, which holds the next free number. `journey.schema.json` is the
Journey file's JSON Schema, which `xtask codegen` writes from
`goodfetch-protocol`'s `Journey` type; never edit it by hand. Each Journey file
names it in its first line, `#:schema ./journey.schema.json`, for editors.

`tasks/seed-<N>.json` is the Proving ground's task list for each seed a
Journey uses, which `just journeys-tasks` writes from the Proving ground's
`tasks --seed N`; never edit it by hand. Run it and commit the result when a
Journey's seed or the Proving ground's tasks change.

`xtask journeys check`, in the Quick suite, holds the catalog to ADR 0063's
rules, reading the task lists rather than starting a Proving ground;
`just journeys-tasks --check`, in the Pre-merge suite, fails when a committed
list differs from the Proving ground's, and `just journeys-tests` holds each
variant's tests to the catalog, from the runners' listings. `just journeys`
runs chosen variants. How to write a Journey and its tests is in `TESTING.md`,
Journeys.
