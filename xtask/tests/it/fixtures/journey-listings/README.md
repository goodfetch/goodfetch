# Journey listings fixture

A workspace with one Journey, J-001, and one test of each of four of its
variants, one per runner: nextest's `journeys::j001::cli`, Vitest's and
Playwright's tagged tests and pytest's `journey` marker. `xtask journeys
listings` lists them with the real runners, and `journeys check --listings`
finds every variant but `J-001@foxhound-cli` tested.
