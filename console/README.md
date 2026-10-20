# Console

The Console, with Foxhound's screens as a package of their own (ADR 0087). Area 12 fills it.

Each package commits the routes its build reads as `routes.json` at its root, `{ "routes": ["/runs", …] }`, which the Feature inventory reads for its `screen:` Features (ADR 0072).
