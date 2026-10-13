# Upgrade and version-mismatch testing

Research for [#271](https://github.com/Jbmanllr/automan/issues/271), part of the map [Goodfetch V1: fully decided test strategy](https://github.com/Jbmanllr/automan/issues/262). Sources were read on 2026-10-13. Links to source code point at each project's default branch on that date unless a tag is named.

## Question

How do comparable systems test upgrades of persisted state and file formats, and Clients and servers running different versions, and what fits Goodfetch's contract rules?

The rules being tested, as decided:

- **ADR 0030.** Each contract format (the JSON-RPC protocol, the plugin protocol, run state and Checkpoints, Plans, Recipe manifests, settings, Schedules, Dataset and Export formats, and others) carries its own `major.minor`. An engine reads every older minor of a format within its major and refuses a newer minor with a message to upgrade. It refuses an older major with a message naming `goodfetch upgrade-files`, and a newer major with a message to upgrade. During the `0.y` previews each preview minor counts as a major. The Usage ledger, the known-certificates store and the Politeness files are exceptions: older engines open their newer versions, which only add, and a *must understand* flag makes an older engine refuse what it cannot read.
- **ADR 0024.** An Attached engine runs the exact engine version its SDK ships with. A Daemon accepts a Client of the same major with the same or an older minor and refuses anything else with a clean error. There is one Daemon per user and per product major.
- **ADR 0055.** Group members of one major and different minors settle on the lower of their two minors, so they can be upgraded in any order.
- **ADR 0030's release gates** already diff the Operation catalog and the settings schema against the previous release of the same major. No ADR requires upgrade fixtures or version-skew tests.

## Summary

The facts a decision rests on:

1. **Two kinds of corpus, used together.** Projects with long-lived file formats keep **committed fixtures**: small files in the repository, one per format version, that the current code must read (Chromium's 79 `version_N.sql` files, Cassandra's legacy SSTables, Kubernetes' serialized objects per release, Temporal's replay histories, Cargo's lockfile tests). They run with ordinary unit tests. **Released binaries** are used for the cases a file cannot show: an old process and a new one talking, rolling upgrades and rollbacks (Kafka, PostgreSQL's buildfarm, Cassandra, containerd, Kubernetes). Those run as system tests or periodic jobs.
2. **A golden check, not just "it opens".** Chromium migrates every committed version up to the current one and requires the same schema as a fresh database. PostgreSQL compares a full dump taken before and after `pg_upgrade`. Kubernetes requires each old fixture to decode and re-encode to identical bytes.
3. **Fixtures are written by the release that owns them and then frozen.** Kubernetes copies its `HEAD` fixtures to `vX.Y.0` at each release. Temporal regenerates histories only for a new feature or a deliberate incompatibility. Kubernetes allows a changed old fixture only as a separate, closely reviewed `after_roundtrip` file.
4. **Refusal is tested cheaply, without old binaries.** Cargo writes a lockfile claiming version `10000000` and checks the exact error. Playwright starts its own server with a version override and checks the mismatch message. Stagehand's cross-version test also checks that an incompatible runtime is refused within 5 seconds rather than at the 60-second timeout.
5. **Skew windows are either narrow and tested, or wide and best effort.** Kubernetes allows `kubectl` within one minor of the API server and runs dedicated skew jobs. Playwright requires the same major and minor. containerd supports only sequential minor upgrades. Docker accepts clients back to API 1.40 (2019) but calls this best effort. When Docker 29.0 raised its minimum to 1.44, its own pull request said the old-version code was "largely untested", and popular tools broke (Portainer's issue drew 439 comments). Docker restored 1.40 in February 2026.
6. **Rolling upgrades of peers are tested under load.** Kafka starts a three-broker cluster on each older release, upgrades one broker at a time while a producer and consumer run, and checks that every acknowledged message was consumed. It also tests rolling back before the upgrade is finalized.
7. **Downgrades usually need the writer to record the oldest version that can still read the file.** SQLite does this with a write version and a read version, Chromium with a "compatible version" number, and Kafka with a feature level that is raised only after the upgrade is finalized. Goodfetch's rule instead refuses every newer minor, except for the Usage-ledger family.
8. **Old binaries come from a fixed source.** Kafka bakes the latest patch of every minor from 2.1 to 4.3 into a test image from a fixed S3 mirror. Cassandra builds the heads of its release branches. containerd downloads previous releases from GitHub Releases at test time, with no checksum check.
9. **Schema diffs complement fixtures.** `buf breaking` checks a schema against a git branch or tag (its strictest category, `FILE`, is the default). containerd freezes its whole gRPC API description into a `.pb.txt` file at every minor. Neither checks stored data.
10. **Direct competitors.** Scrapy 2.19 resumes a job only with the Scrapy version that paused it. Playwright requires the same major and minor between `connect()` and `launchServer()`. Stagehand uses Goodfetch's own rule (same major, server minor ≥ client minor) and tests the current SDK against the last published release.

---

## 1. Old-file corpora kept per release and replayed

### SQLite

- **Compatibility promise.** "The underlying file format for SQLite databases does not change in incompatible ways." Since 3.0.0 (2004-06-18), newer versions read and write every older file. Older versions read newer files "as long as the database does not make use of newer features that are unknown to that older version" ([File Format Changes](https://www.sqlite.org/formatchng.html), updated 2022-09-13). The developers promise to keep the on-disk format backwards compatible through 2050 ([Long Term Support](https://www.sqlite.org/lts.html)).
- **Reader and writer versions in the header.** Bytes 18 and 19 of the header hold a "write version" and a "read version". A file whose write version is newer than the code knows is opened read-only. A file whose read version is newer cannot be read or written ([Database File Format §1.3.3](https://www.sqlite.org/fileformat2.html#file_format_version_numbers)).
- **Cross-version test with old binaries.** `test/backcompat.test` (2010) tests "that the current version of SQLite is capable of reading and writing databases created by previous versions, and vice-versa". Old `testfixture` binaries must be copied in by hand. Each case runs in both directions, and rollback journals and WAL files are shared between the versions ([backcompat.test](https://github.com/sqlite/sqlite/blob/master/test/backcompat.test)). When no old binaries are present, the test prints "WARNING: No historical binaries to test against" and skips itself ([bc_common.tcl](https://github.com/sqlite/sqlite/blob/master/test/bc_common.tcl)).
- **Committed corpus.** Historical fuzzer cases "are collected in a set of database files in the main SQLite source tree" and rerun by `fuzzcheck` on every `make test`. Malformed-file tests corrupt a valid file and check that SQLite returns `SQLITE_CORRUPT` without crashing ([How SQLite Is Tested §4.1.5, §4.2](https://www.sqlite.org/testing.html)).

### PostgreSQL `pg_upgrade`

- **Same-version by default, cross-version on request.** `make check` upgrades from the source tree's version to a new instance of the same version. A cross-version test needs an installed old version (`oldinstall`) and a dump (`olddump`) made from the old version's regression database. Success means the `pg_dumpall` output matches before and after the upgrade ([src/bin/pg_upgrade/TESTING](https://github.com/postgres/postgres/blob/master/src/bin/pg_upgrade/TESTING); [002_pg_upgrade.pl](https://github.com/postgres/postgres/blob/master/src/bin/pg_upgrade/t/002_pg_upgrade.pl)). Each transfer mode (`--copy`, `--link`, `--clone`, `--copy-file-range`, `--swap`) can be tested.
- **Version-specific adjustments live in one module.** `PostgreSQL::Test::AdjustUpgrade` "encapsulates various hacks needed to compare the results of cross-version upgrade tests". It drops objects an old version has but the new one no longer supports, and rewrites known differences in the old and new dumps ([AdjustUpgrade.pm](https://github.com/postgres/postgres/blob/master/src/test/perl/PostgreSQL/Test/AdjustUpgrade.pm)).
- **Released installs, kept by the buildfarm.** The buildfarm's `TestUpgradeXversion` module (client release `REL_21`) saves each branch's installation and regression data after a successful build, then upgrades every older saved branch into the branch under test. It warns that it needs "several GB of extra space". For version 20 and later it skips sources older than 10, because support for upgrading from them was removed ([TestUpgradeXversion.pm](https://github.com/PGBuildFarm/client-code/blob/main/PGBuild/Modules/TestUpgradeXversion.pm)).
- **Supported range.** "pg_upgrade supports upgrades from 9.2.X and later to the current major release" ([PostgreSQL 18 docs](https://www.postgresql.org/docs/current/pgupgrade.html); PostgreSQL 19 was at beta 4 on 2026-09-24).

### Temporal workflow-history replay

- **What it guards.** A workflow's progress is stored as an event history, and a Worker rebuilds its state by replaying that history through the workflow code. "Replay succeeds only if the Workflow Definition is compatible with the provided history". The documented CI practice: download "a representative set of recent open and closed Workflows", run them through replay, and "fail CI if any error is encountered" ([Go testing docs](https://docs.temporal.io/develop/go/testing-suite); [TypeScript testing docs](https://docs.temporal.io/develop/typescript/testing-suite)). The TypeScript SDK raises `DeterminismViolationError` on an incompatible history and `ReplayError` for other failures.
- **Committed histories in the SDKs.** The Go SDK keeps 66 JSON histories in `test/replaytests/`. Eleven are named `bad-*`, and the tests require them to fail with a nondeterminism error ([test/replaytests](https://github.com/temporalio/sdk-go/tree/main/test/replaytests); [replay_test.go](https://github.com/temporalio/sdk-go/blob/main/test/replaytests/replay_test.go)). The TypeScript SDK names some histories after the SDK version that wrote them, such as `lang_flags_replay_correctly_1_9_3.json` ([history_files](https://github.com/temporalio/sdk-typescript/tree/main/packages/test/history_files)).
- **Cross-SDK, cross-version harness.** The `features` repository stores histories at `features/<feature>/history/history.<lang>.<version>.json`. Each run performs three checks: replay the run's own history; replay "all history files with versions <= the current version"; and compare the run's history, with run-specific values removed, against the stored files event by event. Histories are generated once, "on the earliest version the feature is expected to work at", and regenerated only "when a version intentionally introduces an incompatibility" ([temporalio/features README](https://github.com/temporalio/features/blob/main/README.md)). On 2026-10-13 that repository held only 4 such files ([tree](https://github.com/temporalio/features/tree/main/features)).

### Kafka and Cassandra

- **Kafka rolling-upgrade tests.** `upgrade_test.py` starts a 3-broker cluster on each of 3.4 through 4.3 (and on the development branch). A producer and a consumer run while the brokers are upgraded one at a time, then the cluster moves to the latest `metadata.version`. The test checks that "every message acked by the producer was consumed". A second test upgrades without raising `metadata.version`, then rolls back to the starting version under the same check ([upgrade_test.py](https://github.com/apache/kafka/blob/trunk/tests/kafkatest/tests/core/upgrade_test.py)).
- **Kafka's two-phase upgrade.** The upgrade guide has operators replace binaries broker by broker, then "finalize the upgrade" with `kafka-features.sh upgrade --release-version 4.3`. Downgrade "is only possible if there are no metadata changes in the versions between" ([Kafka 4.3 upgrade guide](https://kafka.apache.org/43/getting-started/upgrade/)).
- **Cassandra's committed SSTables.** `LegacySSTableTest` "tests backwards compatibility for SSTables" against committed files for each on-disk format version (`ma`, `mb`, `mc`, `md`, `me`, `na`, `nb`, `oa`, `da`). The files come from a generator test kept `@Ignore`d and are committed with `git add -f`. The format `me` has two sets, from 3.0.25 and 3.11.11, because a serialization change (CASSANDRA-15035) shipped without bumping the format version ([LegacySSTableTest.java](https://github.com/apache/cassandra/blob/trunk/test/unit/org/apache/cassandra/io/sstable/LegacySSTableTest.java); [test/data/legacy-sstables](https://github.com/apache/cassandra/tree/trunk/test/data/legacy-sstables)).
- **Cassandra's upgrade tests between versions.** The in-JVM upgrade tests declare the supported paths as a graph (4.1→5.0, 4.1→6.0, 4.1→7.0, 5.0→6.0, 5.0→7.0, 6.0→7.0) and reject a direct upgrade that is not an edge, naming the supported route ([UpgradeTestBase.java](https://github.com/apache/cassandra/blob/trunk/test/distributed/org/apache/cassandra/distributed/upgrade/UpgradeTestBase.java)).

### Chromium profile migrations

- **One committed schema per version.** `components/test/data/web_database/` holds `version_82.sql` to `version_160.sql`, and `components/test/data/history/` holds `history.22.sql` to `history.70.sql` ([web_database](https://github.com/chromium/chromium/tree/main/components/test/data/web_database); [history](https://github.com/chromium/chromium/tree/main/components/test/data/history)).
- **Golden test.** `VersionXxSqlFilesAreGolden` loads every file from the oldest supported version to the current one, checks the stored version number, migrates it, and requires the resulting schema to equal that of a freshly created database. Each version bump also adds a `MigrateVersionXXToCurrent` test ([web_database_migration_unittest.cc](https://github.com/chromium/chromium/blob/main/components/webdata/common/web_database_migration_unittest.cc)).
- **How fixtures are made.** The fixture is dumped from a real build with a fresh profile, keeping only the version rows. Developers must re-create file N-1 before adding migration N, because "SQL schemas can change without migrations" ([history README](https://github.com/chromium/chromium/blob/main/components/test/data/history/README.md)).
- **Newer files.** Each database records a "compatible version", "the lowest current version embedded in Chrome code that can still use this database". The rule of thumb: "check the compatible version number to see if you can use the file at all. If it's larger than your code is expecting, fail" ([sql/meta_table.h](https://github.com/chromium/chromium/blob/main/sql/meta_table.h)). On open, the Web Data database is wiped when it is older than the oldest supported version or its compatible version is above the code's ([web_database.cc](https://github.com/chromium/chromium/blob/main/components/webdata/common/web_database.cc)). At the profile level, Chrome shows "Your profile can not be used because it is from a newer version of Google Chrome" ([google_chrome_strings.grd](https://github.com/chromium/chromium/blob/main/chrome/app/google_chrome_strings.grd), `IDS_PROFILE_TOO_NEW_ERROR`).

### Kubernetes API objects (comparable practice)

- **A fixture set per release.** `staging/src/k8s.io/api/testdata/` holds `HEAD` plus `v1.34.0` to `v1.37.0` (576 files in `v1.37.0`), in JSON, YAML and protobuf. After every `v1.x.0` release the `HEAD` directory is copied to that version's name. Old fixtures must decode and "round-trip back to serialized form with identical bytes". Adding fields "is *not* expected to require modifications to these fixtures". A changed result goes in a separate `after_roundtrip` file whose update "is exceptional, and requires extremely close review" ([testdata/README.md](https://github.com/kubernetes/kubernetes/blob/master/staging/src/k8s.io/api/testdata/README.md)).
- **The window slides.** On 2026-09-02 the commit adding `v1.37.0` was followed by one deleting `v1.33.0` ([commit history](https://github.com/kubernetes/kubernetes/commits/master/staging/src/k8s.io/api/testdata)).

### Cargo lockfiles (Rust comparable)

- `lockfile_compat.rs` keeps the oldest lockfile format working (`oldest_lockfile_still_works`). It checks that v2 files keep their format when nothing needs updating, and that a lockfile claiming `version = 10000000` fails with "this version of Cargo does not understand this lock file, perhaps Cargo needs to be updated?". It also checks that `version = 5`, the next unreleased format, is refused ([lockfile_compat.rs](https://github.com/rust-lang/cargo/blob/master/tests/testsuite/lockfile_compat.rs)).
- `old_cargos.rs` runs real older Cargo toolchains through rustup, but it is `#[ignore]`d and "intended to be run on a developer system". Its comment says it was last run against 1.0 to 1.51 ([old_cargos.rs](https://github.com/rust-lang/cargo/blob/master/tests/testsuite/old_cargos.rs)).

### Protobuf, `buf breaking` and frozen API descriptions

- **`buf breaking`.** It compares the current schema against a baseline such as `--against 'https://github.com/org/repo.git#tag=v1.0.0'` ([buf inputs](https://buf.build/docs/reference/inputs)). Its four categories, from strictest to most lenient, are `FILE` ("Default"), `PACKAGE`, `WIRE_JSON` ("the recommended minimum level") and `WIRE` ([Rules and categories](https://buf.build/docs/breaking/rules/)). It checks schemas, not stored data.
- **containerd's frozen descriptor.** The whole gRPC API is collected "into a single file". At each minor release the current `next.pb.txt` is renamed after that minor (`1.0.pb.txt`), so "the CI can run" a diff that catches unintended API changes ([RELEASES.md, GRPC API](https://github.com/containerd/containerd/blob/main/RELEASES.md#grpc-api)).
- **JSON Schema equivalent in Rust.** `json-schema-diff` (Sentry, Apache-2.0, 0.1.8, 2026-01-29) diffs two schemas and marks each change `is_breaking`. Its own README calls it "a work-in-progress", for draft-07, to be used "as a best-effort to find obviously breaking changes in CI" ([getsentry/json-schema-diff](https://github.com/getsentry/json-schema-diff)).

### Comparison

| Project | What is kept | Keyed by | Check | Runs |
|---|---|---|---|---|
| SQLite | historical fuzz databases; old binaries supplied by hand | case; binary | no crash, `SQLITE_CORRUPT`; old↔new read and write | `make test`; `backcompat.test` skips without binaries |
| PostgreSQL | regression-database dump from the old version; saved installs | old major | `pg_dumpall` equal before and after | in-tree on request; buildfarm module |
| Temporal | event histories (JSON) | SDK and version that wrote them | replay succeeds; `bad-*` must fail | SDK unit tests; `features` CI |
| Kafka | released tarballs, latest patch per minor | product minor | no acknowledged message lost through a rolling upgrade and downgrade | ducktape system tests |
| Cassandra | SSTables per format; dtest jars per branch | format version; branch | reads legacy data; mixed-version clusters | unit tests; `jvm-dtest-upgrade` |
| Chromium | schema dump per database version | format version | migrated schema equals fresh schema | unit tests |
| Kubernetes | serialized objects per release (window of 4) | product release | identical bytes after a round trip | unit tests |
| Cargo | inline lockfiles per format; old toolchains | format version | reads old; refuses newer with exact text | test suite; old toolchains `#[ignore]`d |

### Fit for Goodfetch

- Goodfetch's run state is a bundled SQLite database (ADR 0002), so SQLite's own file format is not at risk. What changes between minors is Goodfetch's schema inside it and the headers of its formats. That is Chromium's situation: one SQL schema per version, migrated forward.
- ADR 0030 requires every older minor of a major to stay readable. Kubernetes drops fixtures after four releases, which would not satisfy that rule. Chromium and Cassandra keep every supported format version, which does.
- Keying fixtures by format version, as Chromium, Cassandra and Cargo do, means a new fixture only when a format's minor changes. With minors every four weeks and about fourteen formats, the corpus grows only as fast as the formats change.
- The golden checks (Chromium's "same as fresh", PostgreSQL's equal dumps, Kubernetes' identical bytes) need a way to compare an upgraded file with a freshly written one. `insta` (1.49.0, crate updated 2026-10-03, about 111 million downloads) and `snapbox`, which Cargo's tests use for their `str![]` blocks, are the established Rust snapshot libraries ([insta](https://crates.io/crates/insta); [snapbox](https://crates.io/crates/snapbox)).
- ADR 0038 already commits a generated JSON Schema bundle and catalog file. Keeping a copy of the bundle per released minor would be the containerd `.pb.txt` approach. ADR 0030's schema diff against the previous release is the `buf breaking --against tag` approach, which `json-schema-diff` supports only on a best-effort basis.
- Temporal's histories are the closest match to Plans, which "replay without a model", and to resuming a Checkpoint. A Checkpoint written by an older minor and resumed by the current engine is the same problem as an old history replayed by new Worker code.

---

## 2. Version-skew matrices

### Kubernetes

- **Policy.** The three most recent minors are maintained (1.37, 1.36, 1.35). In a highly available cluster, the newest and oldest `kube-apiserver` "must be within one minor version". `kubelet` "must not be newer than kube-apiserver" and "may be up to three minor versions older". `kubectl` is supported "within one minor version (older or newer) of kube-apiserver". The policy also fixes the order in which components are upgraded ([Version Skew Policy](https://kubernetes.io/releases/version-skew-policy/)).
- **The client warns rather than refuses.** `kubectl version` prints "version difference between client … and server … exceeds the supported minor version skew of +/-1" ([skew_warning.go](https://github.com/kubernetes/kubernetes/blob/master/staging/src/k8s.io/kubectl/pkg/cmd/version/skew_warning.go)).
- **Skew jobs.** Periodic jobs run a newer cluster against an older `kubectl`. For example, `ci-kubernetes-e2e-gce-latest-stable2-gci-kubectl-skew` runs every 12 hours with `--extract=ci/k8s-stable2 --extract=ci/latest --skew` ([sig-cli-config.yaml](https://github.com/kubernetes/test-infra/blob/master/config/jobs/kubernetes/sig-cli/sig-cli-config.yaml)). kubeadm's jobs create clusters with an older `kubelet` (`kubelet-1-35-on-1-37`, every 2 to 12 hours) and upgrade between adjacent minors (`upgrade-1-36-1-37`). Both files are "AUTOGENERATED" from a version list by a tool ([kubelet-x-on-y](https://github.com/kubernetes/test-infra/blob/master/config/jobs/kubernetes/sig-cluster-lifecycle/kubeadm-kinder-kubelet-x-on-y.yaml); [upgrade](https://github.com/kubernetes/test-infra/blob/master/config/jobs/kubernetes/sig-cluster-lifecycle/kubeadm-kinder-upgrade.yaml)).
- **Emulated version.** Since 1.32, `--emulated-version` lets a newer control-plane binary behave as an earlier version, "to make upgrades safer". It must be no newer than the binary's own version ([Compatibility Version](https://kubernetes.io/docs/concepts/cluster-administration/compatibility-version/)).

### Docker and containerd

- **Docker API negotiation.** Client and daemon "negotiate the highest version of the API supported by both". On an older API version, "features introduced in later API versions are disabled", and compatibility is "best effort". The version matrix lists Docker 29.8 with API 1.40 to 1.55 ([Engine API versioning](https://docs.docker.com/reference/api/engine/)).
- **Narrowing the window broke users.** In moby #51186 (merged 2025-10-16), Docker 29.0 raised the minimum to 1.44, because code kept for old versions was "largely untested, and a 'best-effort' at most". Docker 29.0 to 29.2 then show minimum 1.44 in the matrix. In moby #52067 (merged 2026-02-27), Docker lowered it back to 1.40; "All the API gates were still in place" ([#51186](https://github.com/moby/moby/pull/51186); [#52067](https://github.com/moby/moby/pull/52067)).
- **containerd.** Its gRPC API has no backwards-incompatible change "without a major version jump". Since containerd 2.0, the API version is separate from the product version (2.4 ships API 1.12). "Upgrades are supported for sequential minor releases", and between consecutive long-term releases ([RELEASES.md](https://github.com/containerd/containerd/blob/main/RELEASES.md)). `TestUpgrade` runs the latest patch of the 2.3 and 2.4 releases, then restarts on the current build. It checks that pods recover, that `exec` into existing containers works, that images are still there, and that metrics parse ([release_upgrade_linux_test.go](https://github.com/containerd/containerd/blob/main/integration/release_upgrade_linux_test.go)).

### Playwright

- **Rule.** "The major and minor version of the Playwright instance that connects needs to match the version of Playwright that launches the browser (1.2.3 → is compatible with 1.2.x)" ([class-browsertype.md](https://github.com/microsoft/playwright/blob/main/docs/src/api/class-browsertype.md); v1.64.0 released 2026-10-07).
- **How it is enforced and tested.** The server reads the client's `Playwright/x.y.z` user agent and returns a boxed "Playwright version mismatch" error that names both versions. If the user agent cannot be parsed, it lets the connection through ([playwrightServer.ts](https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/remote/playwrightServer.ts)). The test starts a server with `PW_VERSION_OVERRIDE: '1.2.3'` and checks the message ([browsertype-connect.spec.ts](https://github.com/microsoft/playwright/blob/main/tests/library/browsertype-connect.spec.ts)).
- **Stated reason.** A maintainer explained that `launchServer`/`connect` are "an internal implementation detail" outside semver, and recommended matching the patch version too ([#16129](https://github.com/microsoft/playwright/issues/16129), 2022).

### Temporal SDKs and server

- **Policy.** SDKs and the server are released independently, and "All SDK versions support all server versions". The server supports upgrades from every version since 1.7.0 and maintains the last three minors ([Versions and support](https://docs.temporal.io/temporal-service/temporal-server#versions-and-support)). Server upgrades must go "one minor version at a time"; compatibility is guaranteed only between consecutive minors ([Upgrade server](https://docs.temporal.io/self-hosted-guide/upgrade-server)). New features still state minimum SDKs: Standalone Nexus Operations in server v1.32.0 (2026-09-11) need Go SDK v1.46.0 and TypeScript SDK v1.20.2 ([v1.32.0 release notes](https://github.com/temporalio/temporal/releases/tag/v1.32.0)).
- **Testing.** The `features` harness exists to "confirm feature behavior across SDK versions" and "verify feature compatibility across different server setups". It runs against a dev server or a given server image, and an SDK version or a local SDK checkout ([features README](https://github.com/temporalio/features/blob/main/README.md)).
- **Worker skew.** Worker Versioning pins a workflow to the Worker deployment version that started it ("Pinned"), or moves it to new versions ("Auto-Upgrade"), in which case it "need[s] to be kept replay-safe manually" ([Worker Versioning](https://docs.temporal.io/worker-versioning)).

### Kafka clients and brokers

- **Policy.** Kafka has a "bidirectional" policy: "new clients can talk to old servers, and old clients can talk to new servers". Clients ask for the broker's supported API versions (since 0.10.0.0, KIP-35) and use "the highest API version supported by both". If none exists, "an error should be reported to the user" ([Kafka 4.3 protocol guide](https://kafka.apache.org/43/design/protocol/)).
- **Tests.** System tests run producers and consumers of every minor from 2.1 to 4.3 against the development broker. Another test runs the current client against brokers of 2.1 to 4.3 and checks which features it finds present or absent ([compatibility_test_new_broker_test.py](https://github.com/apache/kafka/blob/trunk/tests/kafkatest/tests/core/compatibility_test_new_broker_test.py); [client_compatibility_features_test.py](https://github.com/apache/kafka/blob/trunk/tests/kafkatest/tests/client/client_compatibility_features_test.py)).

### Stagehand (direct competitor)

- **Rule.** Its SDKs, browser extension and protocol package have separate versions, and only `protocolVersion` gates compatibility. Stable versions are compatible "when the client and server protocol majors match and the server protocol minor is greater than or equal to the client protocol minor". Pre-releases must match exactly ([packages/protocol/README.md](https://github.com/browserbase/stagehand/blob/main/packages/protocol/README.md); protocol package 2.1.1).
- **Test.** `crossVersionCompat.test.ts` drives "the CURRENT sdk-ts against the extension that shipped in the LAST PUBLISHED" release, in a real Chromium. It fetches the baseline with `npm pack` and computes the expected outcome from the compatibility function rather than hard-coding it. An incompatible runtime must be rejected within 5 seconds, "far below the 60 s init timeout" ([crossVersionCompat.test.ts](https://github.com/browserbase/stagehand/blob/main/packages/sdk-ts/tests/browser-runtime/crossVersionCompat.test.ts), last changed 2026-09-09).

### Comparison

| Project | Window | Who adapts | Refusal | Skew tests |
|---|---|---|---|---|
| Kubernetes | `kubectl` ±1 minor; `kubelet` up to −3; apiservers within 1 | API server serves older clients | warning only (`kubectl`) | periodic jobs, generated from a version list |
| Docker | API 1.40 to 1.55 (29.8, per the matrix) | daemon downgrades its responses | "client version … is too old" | none documented ("best effort") |
| containerd | gRPC API stable within major; sequential minor upgrades | — | — | upgrade from the previous two minors |
| Playwright | same major.minor | none | mismatch error naming both versions | test with a version override |
| Temporal | every SDK with every server; server minors one at a time | per-feature minimum SDK | — | `features` harness across SDK versions and server images |
| Kafka | bidirectional | highest common API version | error when no common version | old clients against the current broker |
| Stagehand | same protocol major, server minor ≥ client minor | — | typed reason, within 5 s | current SDK against the last published release |
| **Goodfetch (ADR 0024)** | same major, Client minor ≤ Daemon minor | Daemon | clean error | none yet |

### Fit for Goodfetch

- Goodfetch's Client rule is Stagehand's rule and Docker's direction: the newer side serves the older one. Where Kubernetes limits the window to one minor, Goodfetch's window covers the whole major: up to about thirteen minors a year at one minor every four weeks.
- Docker shows what happens to a wide window that is not tested: the old-version code decayed, and narrowing the window broke Portainer, Traefik and Testcontainers. Kubernetes and containerd keep their windows narrow and test them.
- Because an SDK always runs the exact engine it ships with (ADR 0024), the SDK and Attached engine pair never has skew. Skew happens on these links:
  - a Client and a Daemon, where the Client is an older SDK, CLI or MCP server;
  - the Browser host and the engine (ADR 0024);
  - the plugin protocol (ADR 0033);
  - Group members' peer link during a rolling upgrade (ADR 0055).

  The Console is embedded in the binary that serves it, so it always matches its Daemon.
- An old Client is a published artifact (npm, PyPI, crates.io, GitHub Releases), as Stagehand's `npm pack` baseline is. Playwright's version override and Stagehand's computed expectation are two ways to test refusal without an old artifact.
- A Group member upgraded "in any order" (ADR 0055) is Kafka's rolling-upgrade case: a cluster on the older minor, Group members replaced one at a time while runs continue, and a check that nothing is lost. Kafka also tests rolling back before the new version is finalized.
- During the `0.y` previews every preview minor counts as a major (ADR 0030). Before 1.0 there is no cross-minor skew to test, only refusal and `goodfetch upgrade-files`.

---

## 3. Building the corpus; downgrade and refusal tests

### From released binaries

- **Kafka** builds a test image that downloads the latest patch of every minor, from 2.1.1 to 4.3.1, from a fixed mirror (`KAFKA_MIRROR="https://s3-us-west-2.amazonaws.com/kafka-packages"`) ([tests/docker/Dockerfile](https://github.com/apache/kafka/blob/trunk/tests/docker/Dockerfile)).
- **Cassandra** clones its own repository and builds a dtest jar from the head of each release branch (`cassandra-4.1`, `cassandra-5.0`, `cassandra-6.0`, `trunk`), not from release tags. It skips a jar that already exists ([.build/run-tests.sh](https://github.com/apache/cassandra/blob/trunk/.build/run-tests.sh), `_build_all_dtest_jars`).
- **containerd** looks up the latest tag of each earlier minor with `git ls-remote` and downloads the static tarball from GitHub Releases at test time. The helper checks the HTTP status and the gzip stream, but not a checksum ([release_upgrade_utils_linux_test.go](https://github.com/containerd/containerd/blob/main/integration/release_upgrade_utils_linux_test.go)).
- **PostgreSQL's buildfarm** keeps installs it built itself ([TestUpgradeXversion.pm](https://github.com/PGBuildFarm/client-code/blob/main/PGBuild/Modules/TestUpgradeXversion.pm)).
- **Kubernetes' skew jobs** extract CI builds named by channel (`ci/k8s-stable2`, `ci/latest`) ([sig-cli-config.yaml](https://github.com/kubernetes/test-infra/blob/master/config/jobs/kubernetes/sig-cli/sig-cli-config.yaml)).
- **SQLite and Cargo** leave old binaries to the developer, and their tests skip themselves (SQLite) or are `#[ignore]`d (Cargo) when the binaries are absent (sources above).

### From committed fixtures

- **Chromium** dumps the schema from a real build, keeps only the version rows, and re-creates file N-1 before adding migration N (above).
- **Cassandra** commits the output of an `@Ignore`d generator test; it notes the generator "needs to be ran alone" ([LegacySSTableTest.java](https://github.com/apache/cassandra/blob/trunk/test/unit/org/apache/cassandra/io/sstable/LegacySSTableTest.java)).
- **Kubernetes** copies `HEAD` to `vX.Y.0` at release time and regenerates `HEAD` with `UPDATE_COMPATIBILITY_FIXTURE_DATA=true` ([testdata/README.md](https://github.com/kubernetes/kubernetes/blob/master/staging/src/k8s.io/api/testdata/README.md)).
- **Temporal** generates a history with `--generate-history` on the earliest version where the feature works, and otherwise leaves it checked in ([features README](https://github.com/temporalio/features/blob/main/README.md)).

### Downgrade tests

- **Kafka** upgrades without finalizing `metadata.version`, rolls the brokers back to the starting version, and checks that no message was lost (above).
- **SQLite's** `backcompat.test` runs each case with the old and new binaries in both roles, including a hot journal left by one version and rolled back by the other (above).
- **Chrome** writes a `Last Version` file into the User Data directory. On a major upgrade it can snapshot the profile, and a later downgrade can restore that snapshot (`kSnapshotRestore`). Downgrades of a minor version need no data processing (`kMinorDowngrade`); other downgrades either wipe the data (`kAdministrativeWipe`) or are unsupported ([downgrade_manager.h](https://github.com/chromium/chromium/blob/main/chrome/browser/downgrade/downgrade_manager.h); [downgrade_manager.cc](https://github.com/chromium/chromium/blob/main/chrome/browser/downgrade/downgrade_manager.cc)).
- **Kubernetes** gives control-plane binaries `--emulated-version` so that a rollback stays possible (above).
- **Recording the oldest reader.** Downgrades work when the writer records the oldest version that can still use the file: SQLite's write and read versions, Chromium's compatible version (for example, an optional column added in version 3 keeps the compatible version at 2), and Kafka's finalized feature level (sources above).

### Refusal tests

- **Cargo** writes a file with a version from the future and checks the whole error message (`lock_from_the_future`, `next_version_is_always_unstable`).
- **Playwright** overrides its own version in the server it starts and checks that both versions are named.
- **Temporal's** Go SDK replays `bad-*` histories and checks the exact nondeterminism message.
- **Stagehand** requires the rejection within a 5-second budget.
- **Cassandra's** test base throws an error that names the supported route when an upgrade skips a step.

### Comparison

| Approach | Examples | Cost | What it catches | What it misses |
|---|---|---|---|---|
| Committed fixtures per format version | Chromium, Cassandra SSTables, Cargo, Kubernetes, Temporal | small files; runs with unit tests | an old file the new reader cannot read or misreads | old process talking to a new one; old reader on a new file |
| Fabricated newer or older headers | Cargo, Playwright override | none | refusal path and message | real newer files |
| Released binaries, file exchanged | SQLite `backcompat`, PostgreSQL `pg_upgrade` | old builds and their storage | both directions; real old writers | live protocol skew |
| Released binaries, live cluster | Kafka, Cassandra dtests, containerd, Kubernetes jobs | system-test infrastructure; periodic | rolling upgrade, downgrade, protocol skew under load | — |
| Schema diff against a release | `buf breaking`, containerd `.pb.txt`, `json-schema-diff` | none | contract changes before release | data already written |

### Fit for Goodfetch

- **Committed fixtures fit the quick tier the owner asked for** (map #262): deterministic, offline, and fast. Each would be a file written by the release that introduced its format minor, never edited afterwards. Kubernetes' exceptional, reviewed `after_roundtrip` file shows how a deliberate change is recorded.
- **Refusal paths need no old binary.** These cases are:
  - a Checkpoint of a newer minor;
  - a Plan of an older major (the message names `goodfetch upgrade-files`);
  - a newer major;
  - a Client of a newer minor or another major;
  - an Operator policy found only for an older major (ADR 0030).

  Each can be produced by writing the header, as Cargo does. The Error code and message can be snapshotted. Stagehand's 5-second budget is a check that refusal is quick.
- **Released-binary tests fit the release tier.** Goodfetch releases are immutable GitHub Releases with attestations (ADRs 0030 and 0032), so an older release's binary or SDK package can be fetched by digest. ADR 0032 denies outbound traffic except to the fixtures and "the pinned artifact mirror". containerd's download at test time, without a checksum, would not fit that rule. Kafka's mirror baked into an image would.
- **Old engines reading newer files.** The Usage ledger, the known-certificates store and the Politeness files promise that older engines open newer versions. That is SQLite's `backcompat.test` "vice-versa" direction, which needs the older binary: committed fixtures cannot test it.
- **Downgrades.** After a newer minor resumes and rewrites a Checkpoint, an older engine refuses it under ADR 0030. Chromium's compatible version, SQLite's write version and Kafka's finalize step are the known ways to allow a rollback when a change only adds. ADR 0030 chose refusal, except for the Usage-ledger family, so a downgrade test would assert refusal.
- **`goodfetch upgrade-files`.** This is PostgreSQL's `pg_upgrade` situation. A fixture of each format at the last minor of the previous major (and, during the previews, of the previous `0.y`) goes through `upgrade-files`, and the result is compared with a golden file. PostgreSQL keeps version-specific workarounds in one module (`AdjustUpgrade`).
- **Missing binaries must not mean a silent skip.** SQLite's `backcompat.test` skips itself and Cargo's `old_cargos.rs` sits `#[ignore]`d (last run against 1.0 to 1.51). Tests that pass because nothing ran are the same problem as the owner's "no flaky tests" requirement.

---

## 4. What users complain about

- **Docker 29.0's minimum API version.** After moby #51186 raised the minimum to 1.44, tools that pinned an older client failed with "client version … is too old":
  - Portainer [#12925](https://github.com/portainer/portainer/issues/12925), 439 comments;
  - Traefik [#12253](https://github.com/traefik/traefik/issues/12253), 112 comments;
  - Nextcloud AIO [#7096](https://github.com/nextcloud/all-in-one/issues/7096), 70 comments;
  - Testcontainers for Java [#11212](https://github.com/testcontainers/testcontainers-java/issues/11212), 40 comments;
  - Watchtower [#2122](https://github.com/containrrr/watchtower/issues/2122), 32 comments.

  Comment counts are as of 2026-10-13. Docker restored 1.40 four months later ([#52067](https://github.com/moby/moby/pull/52067)).
- **Playwright's exact-minor rule.** A user on a 1.18 client and a 1.23 server argued that semver should make minors compatible. The maintainer answered that the remote connection is outside semver ([#16129](https://github.com/microsoft/playwright/issues/16129)).
- **Temporal state after an SDK upgrade.** On 2026-08-10, a user reported "[TMPRL1100] During replay, a matching Timer command was expected" after upgrading the Go SDK from v1.39 to v1.46. That is persisted state from an older version failing under a newer one. The issue was still open on 2026-10-13 ([sdk-go #2537](https://github.com/temporalio/sdk-go/issues/2537)).
- **Chrome profiles after a downgrade.** "Your profile can not be used because it is from a newer version of Google Chrome" is the product's own message for this case (`IDS_PROFILE_TOO_NEW_ERROR`, above).
- **Scrapy job directories.** "The contents of a job directory are an implementation detail of the Scrapy version that wrote them. A job must be resumed with the same Scrapy version that paused it" ([jobs.rst at 2.19.0](https://github.com/scrapy/scrapy/blob/2.19.0/docs/topics/jobs.rst), released 2026-09-10).

### Fit for Goodfetch

- Goodfetch's promise is stronger than Scrapy's: a Checkpoint resumes on any later minor of its major. That promise is what fixtures would test.
- ADR 0030 already answers the Docker lesson for Clients: a window narrows only with a major. Without tests across the whole window, Docker's experience suggests the old-minor paths decay unnoticed.

---

## Unverified

- Whether SQLite's release checklist (about 200 items, run by hand) runs `backcompat.test` with historical binaries. The test skips itself without them.
- How many PostgreSQL buildfarm animals enable `TestUpgradeXversion`.
- How often Kafka's ducktape system tests and Cassandra's `jvm-dtest-upgrade` run in each project's CI (per commit, nightly or before a release). Not checked.
- Whether Chromium refuses or wipes a too-new History database. Only the Web Data path (wiped) and the profile-level message were read.
- Which Docker release first shipped moby #52067. The API matrix shows 29.3 with minimum 1.40; the release notes were not checked.
- Whether Temporal's SDK repositories test against older server versions in CI.
- Whether Stagehand's `crossVersionCompat.test.ts` runs in its default CI, given that it needs network access on first run.
- Whether Crawlee documents what happens to a storage directory written by an older version. Its v1 upgrade guide covers API changes only ([upgrading_to_v1.md](https://github.com/apify/crawlee-python/blob/master/docs/upgrading/upgrading_to_v1.md)).
- Which JSON Schema draft Goodfetch's `schemars` output uses, and so whether `json-schema-diff` (draft-07, work in progress) covers it.
- Whether ADR 0032's "pinned artifact mirror" is meant to hold earlier Goodfetch releases as well as third-party artifacts.
