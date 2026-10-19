# Feature listings fixture

A workspace whose tests declare one Feature per runner, each in its runner's
idiom (ADR 0072): a Rust `Covers:` line, a Vitest tag, a Playwright `covers`
annotation and a pytest `covers` marker. Its Vitest configuration declares
the tags from `features/inventory.json` through test support's
`readFeatureIds`, with `strictTags` on. `xtask journeys listings` lists the
tests with the real runners, and `xtask features scan --listings` reads every
declaration from them.
