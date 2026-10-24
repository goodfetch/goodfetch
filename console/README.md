# Console

The Console, with Foxhound's screens as a package of their own (ADR 0087). Area 12 fills it.

Each package commits the routes its build reads as `routes.json` at its root, `{ "routes": ["/runs", …] }`, which the Feature inventory reads for its `screen:` Features (ADR 0072), and at which the executable serves the Console's page (`xtask console build` keeps it beside the build). A segment that begins with `$` stands for any one segment of the page's path, the route's parameter, as in a run's page, `/runs/$run`, which the executable serves at `/runs/<run-id>`. Collie's are declared in `collie/src/routes.ts`, whose unit test fails when `routes.json` differs.

Each state of a component is a Storybook story beside it (`*.stories.tsx`; `collie/.storybook/`), which the root Vitest configuration's `component-stories-light` and `component-stories-dark` projects render as tests with the a11y addon's axe pass; `pnpm --dir console/collie exec storybook dev` shows them.

A screen that keeps its view in the page's address, as Runs keeps its filter so a view can be shared (ADR 0117), reads and replaces the address's query through the `Address` the build entry passes the shell (`src/address.ts`); its tests pass their own.

Each `screen:` Feature gets a keyboard walk and axe case from `screens.spec.ts`, with no test written for it (ADR 0073). Give each new screen its route and sign-in state in `screens.toml`, or its case fails.

`collie/` is `@goodfetch/console`, Collie's Console: its shell, design system and build entry. `just console-build` (`xtask console build`) builds it into the brotli files Collie's executable embeds (ADR 0057); `pnpm --dir console/collie run messages` rewrites its message catalog, which `just lint` checks.

Its Client, `src/client/`, runs the Collie client package's browser entry over the WebSocket on the Console's own origin, or over Server-Sent Events plus `POST` where a proxy refuses WebSockets, signed in through the link `goodfetch-collie console` opens (`src/sign-in.ts`); its unit tests pass the in-memory adapter of the protocol. It reconnects by itself, sending again its subscriptions and its reads and idempotent writes, never an Effect, while the shell keeps the page read-only under a banner. A call takes an `AbortSignal`, which cancels it on the engine too, as a run's page cancels its waiting `run.progress` read when it closes. `src/generated/operations.ts`, the operation map that types its calls, is `xtask codegen`'s and never edited by hand.
