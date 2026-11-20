## Levels and layout

A test's level is the seam it crosses, not its speed:

- A **unit test** tests one module through its interface, in one process.
- An **integration test** tests one package through its public interface, with real local stand-ins and real processes.
- An **end-to-end test** follows a Journey through a shipped surface.
- The **conformance suites** of the ADRs stay the interface tests of every contract with several implementations.

| | Unit tests | Integration tests |
|---|---|---|
| Rust crate | `#[cfg(test)] mod tests` at the end of the module file, or in `<module>/tests.rs` | `tests/it/main.rs` and its modules |
| TypeScript package | `foo.test.ts` (or `.tsx`) beside `foo.ts` | `<package>/tests/`, importing only the package's own name |
| Python package | `tests/unit/`, mirroring the module paths under `src/` | `tests/integration/`, importing only the public package |

- **A unit test reaches only its module's interface:** the items the module's callers use. Never a private function.
  _Why:_ such tests survive internal refactors, and they test what callers rely on.
- **Product code has no `#[cfg(test)]` branch and no `pub` item added for tests.** If a test needs one, the module is the wrong shape. The one test-only `cfg` in product code is the `sync` modules' `goodfetch_loom` switch ([Concurrency](simulation-model-checking-and-crash-testing.md#concurrency)).
- **One integration-test binary per crate, run with `cargo-nextest`.** A crate splits it into named area harnesses (`tests/<area>/main.rs`, `autotests = false`) only when its pull request shows a localized test edit taking more than 5 seconds to rebuild with one harness.
  _Why:_ each file in `tests/` links its own binary. uv's measurement favours one harness until a suite is large.
- **A compile-fail test (trybuild) lives in its crate's `tests/it/compile_fail/`**, one test per `tests/ui/<folder>`, so it runs in nextest's `compile-fail` test group ([Isolation](injection-and-determinism.md#isolation)).
  _Why:_ trybuild builds every test's cases in one generated project per crate, so two at once can build each other's.
- **Tests of a Product's executable live in its crate's `tests/it`** and find the executable through `CARGO_BIN_EXE_goodfetch-collie` or `CARGO_BIN_EXE_goodfetch-foxhound`, never a guessed path.
  _Why:_ the build directory's layout may change without notice.
- **Foxhound's tests live in Foxhound's crates.** No crate outside `crates/foxhound/` but `xtask` takes a Foxhound crate, even as a dev-dependency, and the workspace-wide `cargo-deny` check refuses one (ADR 0087).
  _Why:_ Collie's tests then build and pass without Foxhound's code.
- **No crate outside `crates/foxhound/` names `goodfetch-protocol`'s `foxhound` module** but `goodfetch-protocol` and `xtask`, and the check refuses a fixture crate that does, through a `use` or a full path ([ADR 0099](../adr/0099-every-surface-spells-each-catalog-name-by-its-own-rule-shapes-come-from-declared-kinds-and-call-is-the-way-down.md)).
  _Why:_ Foxhound's contract types sit in a crate Collie links, so cargo-deny alone cannot keep Collie's code from naming them (ADR 0087).
- **In Collie, only the TLS relay's crate takes BoringSSL** ([ADR 0124](../adr/0124-collies-tls-relay-links-wreqs-boringssl-but-not-wreq-and-forwards-the-browsers-own-http2.md)). Collie's `cargo-deny` check refuses `wreq` and the crates that exist for it under any parent. It refuses `tokio-btls` under any direct parent but the relay's crate, `btls` under any but the relay's crate and `tokio-btls`, and `btls-sys` under any but `btls` (and the relay's crate if it calls the FFI).
  _Why:_ the relay is Collie's one use of BoringSSL; any other would start moving Foxhound's impersonated HTTP into Collie.
- **The declared dependencies of the npm and PyPI packages under `sdks/`, `adapters/` and `integrations/` are checked in the pull-request tier** against `STYLE.md`'s package dependency rule (ADR 0095), as `cargo-deny` checks the crates.
  _Why:_ a Foxhound package depending on `@goodfetch/collie` would install Collie's executable with every Foxhound (ADR 0090).
- **Each Product's executable crate tests its composition** in the Quick suite, in process: exactly one handler for each operation of its Product's catalog, and Foxhound's composition contains all of Collie's (ADR 0087).
  _Why:_ the composition is a hand-kept list, and Foxhound must carry everything Collie carries.
- **Python tests run with `--import-mode=importlib` and `strict = true`** in `[tool.pytest]`.
  _Why:_ tests outside the package run against the installed wheel, as pytest recommends.

### Which Product's executable a test runs

Decided in [ADR 0094](../adr/0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md).

- **A test that runs an executable runs Collie's when Collie carries everything it needs, and Foxhound's otherwise:** once, never once per Product. A test that needs Foxhound anywhere in its setup or steps, such as a rolling upgrade under a Partitioned crawl, runs on Foxhound's.
  _Why:_ Foxhound's executable links Collie's crates, so a test of Collie's covers the code users of either Product run, and the containment checks below cover what Foxhound adds around it.
- **A test that measures an executable itself runs on each Product's:** its size (ADR 0087), its startup benchmarks and its idle memory.
- **Each package is tested against its own Product's executable:** each executable crate's `tests/it` (its CLI, MCP server, exit codes and `--help` tree), and each Product's SDK packages, image and Helm chart.
- **One test build per target serves both Products:** the in-process tests and both executables come from the same workspace invocation. Never add a test build per Product.
  _Why:_ Cargo unifies features across the packages of one invocation, so Collie's executable in a test links the same shared code as Foxhound's, and a second build would compile again every crate whose features differ. Each Product's own `cargo build -p` (ADR 0087) is run from its own bytes by the release before it publishes.
- **Foxhound proves that it contains Collie at the seams where its executable is composed,** never by running Collie's tests again:
  1. its engine composition contains Collie's (ADR 0087's composition test);
  2. a Quick check fails unless every generated contract of Collie's (catalog entries, setting keys, Error codes, events, CLI commands with their flags, MCP tools) is in Foxhound's, the same;
  3. each Collie Journey's `cli` variant also runs on Foxhound's executable, as `J-NNN@foxhound-cli` ([Journeys](journeys.md)).
- **A conformance suite runs on the smallest Product that carries its contract:** browser conformance and the Plugin kinds Collie carries on Collie's executable, Sinks and Fetch backends on Foxhound's. Of the [Session ownership and proxy limit cases](sdk-and-cli-tests.md#sdks), those on a Session a program or an Agent opened and on a Collie Run's proxy limits run on Collie's, and the one on a crawl's Session pool on Foxhound's.

### The Cloud-readiness conformance suite

Decided in [ADR 0068](../adr/0068-the-console-is-tested-in-real-browsers-and-served-by-the-real-binary.md) and [ADR 0096](../adr/0096-the-browser-host-runs-from-either-products-executable-and-goodfetch-cloud-runs-each-run-on-the-product-it-needs.md) (spec #586). Every contract Goodfetch Cloud relies on is a case of this suite, `conformance/cloud-readiness/cases.toml`, with the messages it expects; no prose document stands beside it, and a new cloud-facing contract lands with its case, and its line below, in the same pull request. `goodfetch-conformance` runs it against two kinds of target, each capability naming its kind (`target`), and writes one `conformance` document per target:

- **A gateway, reached by address** (`GatewayTarget`: its `http` or `https` origin, `GatewayAddress`, whose `/protocol` WebSocket is `ws` or `wss`; the token a program presents in its `hello`, as spec #586's duty 8 says; and any certificate its TLS chains to besides Mozilla's roots): the Stub gateway here, plain and behind a TLS terminator, Goodfetch Cloud's gateway from its private repository, the cases unchanged (`GatewaySuite::embedded`, `run_gateway_suite`). Each case opens its own Connections as the Clients it names, a Collie or Foxhound Client (surface `rust`) or a model's Client (surface `mcp`), and tells which engine a run landed on from what that engine answers a Foxhound Client's `run.seeds`: an answer from Foxhound's, `not_in_product` naming Collie as the run's Product and no `install` command from Collie's, which words it so only behind the gateway's mark (ADR 0095), on every channel. `goodfetch-stub-gateway`'s `tests/it` runs it against the Stub gateway, keeping `conformance-cloud-readiness-gateway.json`.
- **An engine driven as a gateway drives one:** its cases join with their tickets.

The gateway cases, with their `covers`:

- `a-collie-client-and-a-foxhound-client-are-each-answered-as-their-product` (`story:14-14`, `story:14-15`): a Collie Client's `hello` is answered as Collie; a Foxhound Client's as Foxhound, or with `not_in_product` by a gateway that runs no Foxhound engine (the Stub gateway given no `--foxhound`).
- `a-client-of-another-protocol-version-is-refused-at-hello` (`story:14-14`): a Client's `hello` of the next protocol minor answers `protocol_version_unsupported`, the Daemon's version rule (ADR 0066).
- `a-models-client-is-held-to-a-models-settings-behind-the-gateway` (`story:14-14`): a model's Client meets the rules it meets locally (ADR 0092), the gateway naming its surface to the engine (`client_surface`): `proxy.server` on its run and on its Session answers `setting_locked`.
- `a-collie-clients-run-lands-on-collies-engine` (`story:14-14`): ADR 0096's row for a Client, needing Collie.
- `a-foxhound-clients-run-lands-on-foxhounds-engine` (`story:14-15`): the same row, needing Foxhound.
- `a-start-naming-a-known-run-id-goes-to-that-runs-engine` (`story:14-17`): a start naming a Run ID the gateway's index holds stays on that run's engine, which answers it with `run_conflict` for its other inputs.
- `a-collie-programs-raw-crawl-start-answers-not-in-product-naming-the-runs-product` (`story:14-16`): a Collie Client's `crawl.start` answers `not_in_product` from Collie's engine, naming the run's Product and no install.
- `a-collie-programs-crawl-setting-answers-not-in-product-naming-the-runs-product` (`story:14-16`): the same for a crawl setting key in a Collie Client's `run.start`.
- `a-collie-programs-crawl-setting-in-a-sessions-settings-answers-not-in-product-naming-the-runs-product` (`story:14-16`): the same for a crawl setting key in the settings of a Session the Collie Client opens on its run, as for every other call that reads a Client's settings.
- `a-collie-client-reads-a-foxhound-run-it-did-not-start` (`story:14-20`): a Collie Client reads a Foxhound Client's run's `run.usage` and `run.seeds`, which Foxhound's engine answers: an engine never narrows its catalog for a Client (ADRs 0095 and 0096), and a call the run's engine lacks is the next case's.
- `a-foxhound-clients-call-a-collie-runs-engine-lacks-answers-not-in-product` (`story:14-20`): a Foxhound Client's `run.seeds` on a Collie Client's run answers `not_in_product` from Collie's engine.

The routing table's Recipe rows (a Recipe that names Foxhound, a Recipe that names Collie held to Collie) join once a start carries a Recipe's manifest.
