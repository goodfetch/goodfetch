# Journeys

The Journey catalog (ADR 0063): one Journey per file, `J-NNN.toml`, and
`catalog.toml`, which holds the next free number. `journey.schema.json` is the
Journey file's JSON Schema, which `xtask codegen` writes from
`goodfetch-protocol`'s `Journey` type; never edit it by hand. Each Journey file
names it in its first line, `#:schema ./journey.schema.json`, for editors.

`xtask journeys check`, in the Quick suite, holds the catalog to ADR 0063's
rules. How to write a Journey is in `TESTING.md`, Journeys.
