# Journey listings fixture

A workspace with one Journey, J-001, and one test of each of three of its
variants, one per runner that runs a surface: nextest's
`journeys::j001::cli`, Vitest's tagged test and pytest's `journey` marker.
Playwright has one test, which carries no Journey. `xtask journeys listings`
lists them with the real runners, and `journeys check --listings` finds every
variant but `J-001@foxhound-cli` tested.
