# Both Products speak one protocol from one Operation catalog marked by Product, and their SDKs share Collie's client packages

ADR 0084 made Collie and Foxhound two Products, Foxhound's executable containing Collie's code, and left to this decision how the protocol, the Operation catalog and the SDKs serve them. Both Products speak one protocol. One catalog lists every operation, each entry marked with the smallest Product that carries it, and each Product's surfaces are generated from its own entries, Foxhound's from both. A Client announces in `hello` the Product it needs, and an engine accepts it when its own Product contains that one. Each language gets a Collie client package with no executable, on which both Products' SDK packages build, so a Foxhound handler's page and a Collie program's page are the same type. The research for the map found, rechecked on 2026-10-15:
- **Products built on a smaller one share its protocol and its types.** Codex's CLI and app server share `codex-app-server-protocol` ([multi-product builds](https://github.com/goodfetch/automan/blob/research/multi-product-builds/rnd/research/2026-10-14-multi-product-builds.md) §5.5); Stagehand v4's three SDKs speak one protocol gated by one `protocolVersion` ([browser-automation products](https://github.com/goodfetch/automan/blob/research/browser-automation-products/rnd/research/2026-10-14-browser-automation-products.md) §2.1). `playwright@1.64.1`'s `index.d.ts` is `export * from 'playwright-core'`, `puppeteer@25.13.0` pins `puppeteer-core` at exactly `25.13.0`, and Crawlee hands its handlers the driver's own page type ([crawl frameworks](https://github.com/goodfetch/automan/blob/research/crawl-framework-products/rnd/research/2026-10-14-crawl-framework-products.md) §4).
- **Separate copies of one set of types break.** Two copies of `@langchain/core` break `instanceof`, which LangChain now overrides because "multiple versions of core are installed" ([langchainjs#11344](https://github.com/langchain-ai/langchainjs/pull/11344)), and two copies of `@crawlee/core` keep two configurations ([naming, packaging and versioning](https://github.com/goodfetch/automan/blob/research/product-packaging-and-names/rnd/research/2026-10-14-product-packaging-and-names.md) §3.2).
- **One source can serve several products' descriptions.** GitHub publishes one OpenAPI document per product (`api.github.com`, `ghec`, each `ghes-3.x`), "automatically kept up to date with the description used to validate GitHub API requests" ([rest-api-description](https://github.com/github/rest-api-description)). MCP's `initialize` negotiates capability objects because servers differ feature by feature ([lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle)), while Playwright's remote `connect()`, Stagehand v4 and Docker exchange versions only (packaging research §4).
- **Browser automation reaches workflow tools too:** Browserbase ships an n8n node, and Pipedream's registry has a Browserbase component ([workflow tools and loaders research](https://github.com/goodfetch/automan/blob/research/sinks-integrations-catalog/rnd/research/2026-10-11-sinks-integrations/b1-integrations.md)).

Decided in [Protocol, Operation catalog and SDKs across two products](https://github.com/goodfetch/automan/issues/458), on the map [Goodfetch products: Collie and Foxhound as separately installable products](https://github.com/goodfetch/automan/issues/447), by the agent on the owner's delegation of 2026-10-14 ([rounds 1 to 3](https://github.com/goodfetch/automan/issues/458#issuecomment-6181102575), [grilling record](https://github.com/goodfetch/automan/issues/458#issuecomment-6181108777)). It builds on [ADR 0088](0088-collie-carries-every-capability-except-those-only-many-pages-need.md) (Collie's capability set), [ADR 0090](0090-products-are-named-goodfetch-collie-and-goodfetch-foxhound-and-share-one-product-version.md) (names and the Product version) and [ADR 0087](0087-each-product-is-an-executable-crate-that-links-only-its-crates-and-has-its-own-size-budget.md) (how the Products are built).

## Decision

### One protocol

- **Both Products' engines speak one JSON-RPC protocol,** defined in `goodfetch-protocol` (ADR 0038) and versioned by one `major.minor` (ADR 0030). Foxhound's engine serves Collie's operations unchanged.
- A protocol per Product would put two version lines on one wire, bump both for any change to an operation of Collie's, and make the Console, the Browser host and Goodfetch Cloud speak two. One line costs only this: an operation added for Foxhound also raises the minor a Collie Client sees.

### One Operation catalog, marked by Product

- **Each operation names the smallest Product that carries it:** `collie` for what both Products carry, since Foxhound contains Collie, or `foxhound` for what only Foxhound carries. Nothing is Collie's alone: Foxhound's executable carries everything Collie's does by construction (ADR 0087), so the mark is one value, not a list.
- **The same mark is on every other contract a surface is generated from,** beside the catalog in `goodfetch-protocol`: events, Error codes, the settings schema's keys, the plugin protocol's Plugin kinds and Hook stages, CLI commands and MCP profiles, and every value Foxhound adds to a type both Products share, such as a Usage kind, a Limit scope or a Notification event (a Partitioned crawl's Limit scope, "records changed"). The settings schema's list of the Products that honour a key (ADR 0087) is read from it. Below, an *entry* is an operation or any of these.
- **A value only Foxhound carries is treated as ADR 0087 treats Foxhound's setting keys:** a Collie engine refuses it with `not_in_product` where a program, a flag, a form or a Recipe sets it, and lists it as Foxhound's where a shared file holds it; a Collie Client that reads one from a Foxhound engine keeps it as an unknown value (ADR 0079, G3).
- **A Product's catalog is its own operations plus those of the Product it contains,** and the same holds for every other marked contract. ADR 0087's generated table of the names only Foxhound carries, and each executable crate's test of one handler per operation of its Product's catalog, come from the mark.
- **Names carry no Product:** `crawl.start`, never `foxhound.crawl.start`. An entry may widen from Foxhound to Collie in a minor, which adds it to Collie; narrowing an entry out of Collie removes it there, so it is a breaking change, only in a major (ADR 0030).
- **One bundle and one catalog file stay committed** (ADR 0038). Each generator reads the projection of the Product it generates for.

### `hello`

- **A Client's `hello` adds the Product it needs and its own Product version,** both required. The engine's reply adds the engine's Product and Product version, which the connection's `engine` value reports (ADR 0078).
- **No capability list.** Within a Product, every engine of one version has the same catalog, because a capability is a setting, never a Cargo feature (ADRs 0038 and 0087). What varies within a Product (installed Plugins, Operator policy, installed browsers) is read through operations, as before.
- **The version check stays on the protocol's `major.minor`,** under ADR 0024's rule as amended by ADR 0066. Product versions are reported, not compared.

### Which Client reaches which engine

- **An engine accepts a Client whose needed Product its own contains.** A Foxhound engine accepts Clients of both Products; a Collie engine refuses a Client that needs Foxhound.
- **The refusal is `not_in_product`** (ADR 0087: an `UnavailableError`, not retryable), sent as the reply to `hello` before any other message, and the connection then closes, as a refused version does (ADR 0066). Its details name the engine's Product and version and the Product the Client needs; its `fix` says to connect to an engine of that Product or to run with it. A Foxhound program on a Collie engine therefore fails at `connect()`, never at its first crawl call (ADR 0078, G6).
- **The Product check comes before the version check,** since its fix (another Product's engine) makes the version moot.
- **An engine never narrows its catalog per connection.** An accepted Client may call every operation of the engine's Product, a raw catalog call from a Collie Client (G8) included; a call outside the engine's Product is refused with `not_in_product` (ADR 0087).
- **Who announces which Product:**
  - each SDK, its own;
  - the CLI and the MCP server, when they are Clients of a Daemon (ADR 0024), their executable's;
  - the Console, its engine's;
  - an engine reaching a Browser host, Collie, since a Browser host's work is Collie's (ADR 0088);
  - an Integration, the Product it is built for (below);
  - the conformance harness, whichever Product a case tests.
- **How Goodfetch Cloud passes a Client's Product to the engine it routes to** is [Which executables the Browser host and Goodfetch Cloud's engines run](https://github.com/goodfetch/automan/issues/498)'s.
- **Every Group member of a Daemon group runs the same Product.** The peer protocol's handshake carries it and requires equality, not containment: runs move between Group members (ADR 0055), a Collie member could not resume a Foxhound run, and a Foxhound member placed beside Collie ones would leave its Foxhound runs nowhere to go. A Daemon of the other Product is refused with `not_in_product`, whose details name both Products.

### The SDK packages

- **Each language has a Collie client package with no executable,** on which both Products' SDK packages build:

  | | TypeScript | Python | Rust |
  |---|---|---|---|
  | Collie client package | `@goodfetch/collie-client` | `goodfetch-collie-client`, imported as `goodfetch_collie_client`, one pure `py3-none-any` wheel | `goodfetch-collie-client` |
  | Collie | `@goodfetch/collie` | `goodfetch-collie` | `goodfetch-collie` |
  | Foxhound | `@goodfetch/foxhound` | `goodfetch-foxhound` | `goodfetch-foxhound` |

  - It is named after Collie, whose code it is, and "client" because it is the Client side with no engine inside. It carries the Product version and falls under semver (ADR 0030).
  - It holds the hand-written runtime (transports, `hello`, framing, request correlation, credits, cancellation, event resumption, file-backed bodies), the generated layer of Collie's catalog, and every value a Collie connection hands out: Runs, Sessions, Pages, Locators, results, the error group classes with the classes of Collie's Error codes, and Collie's settings types.
- **Each Product's package adds its connection type and finds its own executable,** reading only its own Product's engine-path variable (ADR 0090), and depends on the client package at exactly its own version.
  - `@goodfetch/collie` holds `Collie`, which finds Collie's executable through the platform packages it depends on itself, re-exports the client package, and keeps its `bin` and testing kit (ADR 0090).
  - `@goodfetch/foxhound` holds `Foxhound`, the generated layer of Foxhound's own entries and the objects only Foxhound hands out, re-exports the client package, and keeps its own `bin` and testing kit.
  - Python's and Rust's packages are laid out the same way.
- **Values are the same types in both SDKs.** A Foxhound handler's page is the client package's Page, as it is in a Collie program, so a helper typed for one accepts the other, and an `except` or a `match` on a Collie error class catches it from either connection.
- **Programs import only from their Product's package.** Only a library meant to serve both Products imports the client package itself (S2), and takes it as a peer dependency on npm and a semver range on PyPI and crates.io, so the program's one copy is used.
- **A program that uses both Products' SDKs uses them at one Product version.** Each Product's package pins the client package exactly, because the client package speaks the protocol version its Attached engine must accept (ADR 0024). pip and uv therefore refuse two Product versions in one environment, and Cargo refuses two within one major. Across majors, or two `0.y` previews, which count as majors (ADR 0030), Cargo builds two copies of the client crate, as npm nests two copies of the client package, and their values are not one type. The Products' executables and their installs still need not match (ADR 0090).
- **The connection type stays out of the client package.** It would have to find a Product's executable without depending on that Product's package, which pnpm's strict layout and Yarn's Plug'n'Play refuse, or through registration at import, against G2 and G4.

### What hangs from each connection

- **The connection is the Product:** `Collie` or `Foxhound` (ADR 0090), with no `collie` or `foxhound` property. ADR 0078's `gf.collie.start(…)` becomes `collie.start(…)`; the verbs are [The Run in code](https://github.com/goodfetch/automan/issues/478)'s.
- **A Foxhound connection carries every member of a Collie connection under the same names,** plus Foxhound's own, so moving a program from Collie to Foxhound changes its import and its connection type only.
- **The members both connections share are declared once, in the client package,** as the type a function takes to accept either connection: an interface in TypeScript, a base class in Python, a trait in Rust. `Foxhound` cannot extend a `Collie` that lives in Collie's own package. The type's name is [Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471)'s.

### The Rust crates

- **Three public crates:** `goodfetch-collie-client`, the client with Collie's values and no engine, a further crate of Collie's in ADR 0090's `goodfetch-<product>-<part>` form; `goodfetch-collie`, Collie's Rust SDK, with `Collie` and the linked Collie engine; and `goodfetch-foxhound`, with `Foxhound`, Foxhound's values and the linked Foxhound engine.
- **Both SDK crates depend on `goodfetch-collie-client` at exactly their own version,** so their values are one type, as in the other languages. Each links its Product's engine composed as ADR 0087 composes that Product's executable, Foxhound's starting from Collie's composition; which crates hold the compositions is ADR 0087's.
- **Foxhound's entries stay out of Collie's crates' public API.** The Rust SDKs reuse `goodfetch-protocol`'s types with no generation (ADR 0038), and their errors expose its Error codes (ADR 0079), so each SDK crate re-exports only its own Product's entries. How `goodfetch-protocol` lays its types out for that is [Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471)'s; a check of each crate's public items against its Product's catalog enforces it.
- **No SDK crate compiles an executable's own dependencies,** V8 first (ADR 0051). Cargo compiles every non-optional dependency of a package whichever target needs it ([cargo#1982](https://github.com/rust-lang/cargo/issues/1982), open since 2015), so if an SDK crate also carries its Product's executable as a binary target, that target's own dependencies sit behind a feature its `[[bin]]` requires, off by default. Whether it does, and how `cargo install` installs the executable, is [Release, signing and testing of two products](https://github.com/goodfetch/automan/issues/459)'s (ADR 0090).

### Surfaces per Product

- **Each Product's surfaces are generated from its own catalog, and Foxhound's carry every entry of Collie's under the same name.**
- **CLI and MCP server:** one per executable (ADR 0090), each from its own catalog. Foxhound's executable carries every command of Collie's unchanged.
- **Agent skills:** one source per topic, rendered for each Product that carries it, with its executable's name and its catalog. Collie's topics ship with both Products, Foxhound's only with Foxhound.
- **Code mode and Script Hooks:** their globals are the namespaces of their Product's catalog (ADR 0051).
- **MCP profiles carry the same mark.** A Collie profile offers the same tools in both Products' servers, so an MCP host's configuration moves from Collie to Foxhound by changing the executable only. Foxhound's own profiles may combine Collie's and Foxhound's operations. Each Product's default profile is one of its own, and Code mode stays opt-in in both (ADR 0088). Which profiles exist is area 11's spec.
- **Framework adapters** (the Vercel AI SDK, LangChain and LangGraph in TypeScript and Python, Mastra, ADR 0011) exist once per Product, named `@goodfetch/<product>-<target>` and `goodfetch-<product>-<target>` (ADR 0090), each depending on its Product's SDK package. Foxhound's offer Collie's tools too.
- **Integrations** (ADR 0027):
  - each workflow tool (n8n, Zapier, Make, Pipedream, Activepieces, Power Automate) gets one Integration per Product, n8n's named `@goodfetch/n8n-nodes-<product>` (ADR 0090). Collie's start runs and Recipes and return their results, with Notifications as triggers; Foxhound's add crawls, Records and the record-range triggers;
  - the Airbyte source and the LangChain and LlamaIndex Dataset loaders read Datasets, so they are Foxhound's only; the LangChain loader sits in Foxhound's LangChain package (ADR 0070).
- **Reference pages mark each operation's Product.** How the docs site presents two Products is [Docs site and positioning of two Products](https://github.com/goodfetch/automan/issues/499)'s.

### Where the packages live

- **`sdks/typescript/` and `sdks/python/` each hold `collie-client/`, `collie/` and `foxhound/`.** The Rust crates sit on their Product's side of the crate graph, `goodfetch-foxhound` under `crates/foxhound/` (ADR 0087). `adapters/` and `integrations/` hold one folder per Product.
- **CI checks the declared dependencies of the npm and PyPI packages** under `sdks/`, `adapters/` and `integrations/` in the pull-request tier, as cargo-deny checks the crates (ADR 0087): Foxhound's packages depend on no Collie package but the client package, so never on one carrying Collie's executable (ADR 0090); Collie's depend on no Foxhound package; the client package depends on neither Product's package.

## Considered Options

- **A protocol per Product:** rejected, above.
- **A catalog per Product, Foxhound's including Collie's by reference:** rejected. Two files to keep consistent and an inclusion rule, for what one mark gives.
- **One unmarked catalog, with what an engine carries found at run time:** rejected. Collie's surfaces would show Foxhound's operations, against ADR 0084.
- **The Product in operation names** (`foxhound.crawl.start`): rejected. Moving an operation from Foxhound to Collie, as ADR 0088 foresees for Paid fetches, would rename it, which is breaking.
- **A capability list in `hello`,** as MCP and LSP negotiate: rejected. Within a Product it would be a second source of the catalog that could disagree with the first.
- **Engines that accept every Client and refuse per call:** rejected. A Foxhound program on a Collie engine would connect and fail at its first crawl call, against G6.
- **Each Product's engine accepting only its own Product's Clients:** rejected. A Collie program could not use a Foxhound Daemon or a Goodfetch Cloud engine that runs Foxhound, though Foxhound contains Collie.
- **Comparing Product versions in `hello`:** rejected. The protocol carries its own version (ADR 0030), and a Product minor that leaves the protocol unchanged would refuse Clients for nothing.
- **Each Product's SDK built from shared sources, with nothing shared at install,** as Ruff and ty install their shared code twice: rejected. `goodfetch_foxhound.Page` and `goodfetch_collie.Page` would be different types, so a helper typed for one fails pyright and rustc on the other, an `except` for one error class misses the other's copy, and every library for pages would need a version per Product.
- **Foxhound's npm and PyPI packages depending on Collie's:** rejected. They would install Collie's executable with every Foxhound, which ADR 0090 forbids. How Foxhound's crates use Collie's is ADR 0087's.
- **The connection type in the client package:** rejected, above.
- **A Collie facet on Foxhound's connection** (`foxhound.collie.start(…)`): rejected. One more concept and two spellings of starting a Run (S2). **A Foxhound connection without Collie's members:** rejected. A Foxhound program that logs in once before crawling would need Collie's package too.
- **One Integration per workflow tool for both Products:** rejected. It would have to be named after Goodfetch, which names nothing installed (ADRs 0084 and 0090). **Workflow-tool Integrations for Foxhound only:** rejected. Collie's users would lack a surface browser-automation vendors already ship.
- **MCP profiles chosen per server:** rejected. One profile name could then mean two tool sets.

## Consequences

- **Amends ADR 0011.** Each operation, and each contract generated beside it, names its Product, each Product's surfaces are generated from its own catalog, Foxhound's carrying Collie's entries under the same names, and MCP profiles and Framework adapters are per Product. A note sits under its opening.
- **Amends ADR 0024.** `hello` carries the Product a Client needs, and an engine refuses a Client whose Product it does not contain; each Product's Rust SDK crate links its own engine, and the Collie client crate links none. Notes sit under its opening and under its protocol-version consequence.
- **Amends ADR 0027.** Workflow-tool Integrations exist per Product; the Airbyte source and the Dataset loaders are Foxhound's. A note sits under its list of Integrations.
- **Amends ADR 0030.** Semver covers each Product's catalog: an entry leaving Collie's is breaking for Collie, one joining it is additive. The client packages are released with every Product version, and the API breakage checks cover them. A note sits under "What semver covers from 1.0".
- **Amends ADR 0032.** `sdks/typescript/` and `sdks/python/` hold the client package and each Product's package, `adapters/` and `integrations/` one folder per Product, and the pull-request tier checks the declared dependencies of the npm and PyPI packages under them. A note sits under the tree.
- **Amends ADR 0038.** Each operation in `goodfetch-protocol` also names its Product; each SDK package generates only its Product's own entries, the hand-written runtime living in the client package; and each SDK generates error classes for its Product's Error codes, a code from outside them arriving as its group's class (ADR 0079). Notes sit under its opening and under "What is generated and what is hand-written".
- **Amends ADR 0078.** The connection is the Product, a Foxhound connection carries Collie's members, the values below it come from the client package, and the connection's `engine` value also reports the engine's Product and Product version. A note sits under its entry point.
- **Amends ADR 0087.** Collie publishes two crates and Foxhound one, and the generated names of what only Foxhound carries come from the mark and cover the values Foxhound adds to shared types. Notes sit under its items on the published Rust crates and "How an engine knows".
- **Amends ADR 0088.** The catalog's mark, `hello`, MCP profiles and Integrations per Product are set here. A note sits under its item on Collie's Operation catalog.
- **Amends ADR 0090.** Its packages gain the Collie client packages; each executable offers its catalog's commands; adapters and workflow-tool Integrations exist per Product; a Foxhound connection carries Collie's members; a program using both Products' SDKs uses them at one Product version, while the Products' executables and their installs still need not match; and one SDK accepts the other's values. Notes sit under its packages table and its items on shared commands, adapters and Integrations, the connection's name, "One version does not tie installs together", the packages each Product carries, and two SDKs in one program.
- **Adds to ADR 0055:** every Group member of a Daemon group runs one Product.
- **Adds to ADR 0069:** the Scripted engine also answers `hello` as either Product.
- **Earlier ADRs.** In an ADR written before this one, the Operation catalog a surface exposes is that surface's Product's catalog: Code mode's globals (ADR 0051), the MCP projection (ADR 0065) and the generated-surface checks (ADR 0070) are per Product.
- **Error codes.** `not_in_product` (ADR 0087) is also the refusal of `hello` and of the peer protocol's handshake.
- **Glossary.** *Operation catalog* says each operation is marked with its Product and each Product's surfaces expose that Product's operations; *Framework adapter* offers a Product's operations. No new term: the client package is a package, not domain language.
- **`STYLE.md`.** The hand-written runtime lives in each language's client package; each TypeScript and Python SDK package generates only its own Product's entries, and each Rust SDK crate re-exports only its own; the npm and PyPI packages' dependency rule; the documentation rules of the public crates cover `goodfetch-collie-client`.
- **`TESTING.md`.** The Product check of `hello` and of the peer handshake is tested as version refusals are; the generated-surface check refuses Foxhound's entries on Collie's surfaces and Collie's Rust crates; completeness tests look in each Product's package; the MCP projection is snapshotted per Product and profile; Product conformance cases and type tests run in every SDK. How each Product's surfaces and the skew between Products are tested stays [Release, signing and testing of two products](https://github.com/goodfetch/automan/issues/459)'s.
- **ADR 0037's matrix.** SDK-01, SDK-02, AI-06 and pain row 3 cite this ADR. Areas 1, 3 and 11 list it.
- **Areas.**
  - Area 1 (Repository, CI and release) builds the package layout, the client packages' release and the dependency check.
  - Area 3 (Engine runtime) builds the Product mark in `goodfetch-protocol` and the Product check of `hello` and of the peer protocol.
  - Area 11 (Client surfaces) builds the three packages per language, the connection types, and every surface per Product.
- **Downstream.**
  - [Collie and Foxhound installed on one machine](https://github.com/goodfetch/automan/issues/486) decides what an SDK does when the Daemon it finds refuses its Product, as ADR 0024 restarts an idle Daemon on a refused version.
  - [Which executables the Browser host and Goodfetch Cloud's engines run](https://github.com/goodfetch/automan/issues/498) decides whether Goodfetch Cloud routes a connection by the Product it announces.
  - [Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471) names the shared connection type.
  - [API sketch: type-checked stubs and golden scenarios](https://github.com/goodfetch/automan/issues/483) replaces the sketch's `goodfetch` placeholder with the three packages per language.
  - [Apply the product-line decisions to the affected specs and tickets](https://github.com/goodfetch/automan/issues/460) carries this into the specs and tickets.
