# Console

The Console, with Foxhound's screens as a package of their own (ADR 0087). Area 12 fills it.

Each package commits the routes its build reads as `routes.json` at its root, `{ "routes": ["/runs", …] }`, which the Feature inventory reads for its `screen:` Features (ADR 0072), and at which the executable serves the Console's page (`xtask console build` keeps it beside the build). Collie's are declared in `collie/src/routes.ts`, whose unit test fails when `routes.json` differs.

Each state of a component is a Storybook story beside it (`*.stories.tsx`; `collie/.storybook/`), which the root Vitest configuration's `component-stories-light` and `component-stories-dark` projects render as tests with the a11y addon's axe pass; `pnpm --dir console/collie exec storybook dev` shows them.

Each `screen:` Feature gets a keyboard walk and axe case from `screens.spec.ts`, with no test written for it (ADR 0073). Give each new screen its route and sign-in state in `screens.toml`, or its case fails.

`collie/` is `@goodfetch/console`, Collie's Console: its shell, design system and build entry. `just console-build` (`xtask console build`) builds it into the brotli files Collie's executable embeds (ADR 0057); `pnpm --dir console/collie run messages` rewrites its message catalog, which `just lint` checks.

Its Client, `src/client/`, runs the Collie client package's browser entry over the WebSocket on the Console's own origin, signed in through the link `goodfetch-collie console` opens (`src/sign-in.ts`); its unit tests pass the in-memory adapter of the protocol. `src/generated/operations.ts`, the operation map that types its calls, is `xtask codegen`'s and never edited by hand.
