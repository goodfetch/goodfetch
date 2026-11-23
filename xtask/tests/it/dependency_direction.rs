//! The dependency-direction check: `goodfetch-test-support` depends on no
//! Goodfetch crate and is only ever a dev-dependency (ADR 0062);
//! `goodfetch-test-client` depends on no Goodfetch crate but
//! `goodfetch-protocol` and `goodfetch-test-support`, is only ever a
//! dev-dependency and is never taken by `goodfetch-protocol` (ADR 0062's
//! note of 2026-11-07); a test adapter crate, such as
//! `goodfetch-simulation` (ADR 0060) or `goodfetch-scripted-model`, is only
//! ever a dev-dependency, has `publish = false`, and no crate among its
//! dependencies names it under its `src/`, where its unit tests live, while
//! their integration tests and any other crate's unit tests may (ADR 0125;
//! the fixture's `goodfetch-model-provider` and `goodfetch-repair`); the Proving
//! ground depends on no engine crate, nor on `goodfetch-conformance` or
//! `goodfetch-protocol`; `goodfetch-conformance` depends only on
//! `goodfetch-protocol` (ADR 0032, STYLE.md); `xtask` builds no Goodfetch
//! crate but `goodfetch-protocol` (ADR 0072); `goodfetch-runtime` depends on
//! no Goodfetch crate, and the test helpers and the Proving ground take it
//! (ADR 0081). The fixture's members all take
//! `goodfetch-workspace-hack`, which no rule counts (ADR 0094).
//! Covers: story:1-107
//! Covers: story:1-67

use rstest::rstest;

use crate::workspace::{Workspace, finding};

/// The fixture as it is passes, the Scripted model taken by its
/// dependency's integration tests and by another crate's unit tests
/// included.
#[test]
fn test_support_taken_only_as_a_dev_dependency_passes() {
    let workspace = Workspace::from_fixture("dependency-direction");

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::normal_dependency("engine-normal-dependency.toml")]
#[case::build_dependency("engine-build-dependency.toml")]
fn test_support_outside_dev_dependencies_fails(#[case] manifest: &str) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        &format!("dependency-direction-cases/{manifest}"),
        "crates/goodfetch-engine/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("test-support-not-dev-only", "goodfetch-engine")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn test_support_depending_on_a_goodfetch_crate_fails() {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        "dependency-direction-cases/test-support-depends-on-engine.toml",
        "crates/goodfetch-test-support/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "test-support-depends-on-goodfetch",
            "goodfetch-test-support"
        )]
    );
    assert_eq!(outcome.code, 1);
}

/// Every crate that builds a runtime takes `goodfetch-runtime`, the test
/// helpers and the Proving ground included, so it depends on no Goodfetch
/// crate itself (ADR 0081).
#[test]
fn runtime_depending_on_a_goodfetch_crate_fails() {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        "dependency-direction-cases/runtime-depends-on-engine.toml",
        "crates/goodfetch-runtime/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("runtime-depends-on-goodfetch", "goodfetch-runtime")]
    );
    assert_eq!(outcome.code, 1);
}

/// Covers: story:2-42
#[rstest]
#[case::engine("proving-ground-depends-on-engine.toml")]
#[case::conformance("proving-ground-depends-on-conformance.toml")]
#[case::protocol("proving-ground-depends-on-protocol.toml")]
#[case::protocol_as_a_build_dependency("proving-ground-build-depends-on-protocol.toml")]
#[case::engine_as_a_dev_dependency("proving-ground-dev-depends-on-engine.toml")]
#[case::test_client_as_a_dev_dependency("proving-ground-dev-depends-on-test-client.toml")]
fn proving_ground_depending_on_a_goodfetch_crate_fails(#[case] manifest: &str) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        &format!("dependency-direction-cases/{manifest}"),
        "proving-ground/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "proving-ground-depends-on-goodfetch",
            "goodfetch-proving-ground"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn test_client_depending_on_more_than_the_protocol_and_test_support_fails() {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        "dependency-direction-cases/test-client-depends-on-engine.toml",
        "crates/goodfetch-test-client/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "test-client-depends-on-more-than-protocol",
            "goodfetch-test-client"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::normal_dependency("engine-normal-depends-on-test-client.toml")]
#[case::build_dependency("engine-build-depends-on-test-client.toml")]
fn test_client_outside_dev_dependencies_fails(#[case] manifest: &str) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        &format!("dependency-direction-cases/{manifest}"),
        "crates/goodfetch-engine/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("test-client-not-dev-only", "goodfetch-engine")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::normal_dependency("engine-normal-depends-on-simulation.toml")]
#[case::build_dependency("engine-build-depends-on-simulation.toml")]
fn simulation_outside_dev_dependencies_fails(#[case] manifest: &str) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        &format!("dependency-direction-cases/{manifest}"),
        "crates/goodfetch-engine/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("test-adapter-not-dev-only", "goodfetch-engine")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn scripted_model_outside_dev_dependencies_fails() {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        "dependency-direction-cases/protocol-depends-on-scripted-model.toml",
        "crates/goodfetch-protocol/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("test-adapter-not-dev-only", "goodfetch-protocol")]
    );
    assert_eq!(outcome.code, 1);
}

/// A test adapter is never published: `publish = false`, not left unset nor
/// naming a registry.
#[rstest]
#[case::publish_unset("scripted-model-publish-unset.toml")]
#[case::publish_to_a_registry("scripted-model-publish-to-registry.toml")]
fn test_adapter_without_publish_false_fails(#[case] manifest: &str) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        &format!("dependency-direction-cases/{manifest}"),
        "crates/goodfetch-scripted-model/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "test-adapter-published",
            "goodfetch-scripted-model"
        )]
    );
    assert_eq!(outcome.code, 1);
}

/// A unit test of a crate the adapter depends on, here through
/// `goodfetch-model-provider`, would link two copies of that crate: the
/// crate's `src/` never names the adapter, under its own name or the one
/// its manifest gives it.
#[rstest]
#[case::by_its_name(
    "engine-dev-depends-on-scripted-model.toml",
    "#[cfg(test)]\nmod tests {\n    use goodfetch_scripted_model::ScriptedModel;\n}\n"
)]
#[case::by_its_manifests_name(
    "engine-dev-depends-on-scripted-model-renamed.toml",
    "#[cfg(test)]\nmod tests {\n    use scripted::ScriptedModel;\n}\n"
)]
fn test_adapter_in_a_unit_test_of_its_dependency_fails(
    #[case] manifest: &str,
    #[case] source: &str,
) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        &format!("dependency-direction-cases/{manifest}"),
        "crates/goodfetch-engine/Cargo.toml",
    );
    workspace.write(
        "crates/goodfetch-engine/src/tests.rs",
        format!("//! Fixture engine tests.\n\n{source}").as_bytes(),
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "test-adapter-in-unit-tests-of-its-dependency",
            "crates/goodfetch-engine/src/tests.rs"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn protocol_taking_test_client_fails() {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        "dependency-direction-cases/protocol-dev-depends-on-test-client.toml",
        "crates/goodfetch-protocol/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("protocol-takes-test-client", "goodfetch-protocol")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn conformance_depending_on_more_than_the_protocol_fails() {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        "dependency-direction-cases/conformance-depends-on-engine.toml",
        "crates/goodfetch-conformance/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "conformance-depends-on-more-than-protocol",
            "goodfetch-conformance"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::normal_dependency("xtask-depends-on-engine.toml")]
#[case::build_dependency("xtask-build-depends-on-engine.toml")]
fn xtask_building_a_goodfetch_crate_fails(#[case] manifest: &str) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.add_fixture_file(
        &format!("dependency-direction-cases/{manifest}"),
        "xtask/Cargo.toml",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("xtask-builds-a-goodfetch-crate", "xtask")]
    );
    assert_eq!(outcome.code, 1);
}

const ENGINE_ROOT: &str = "crates/goodfetch-engine/src/lib.rs";

/// Outside `crates/foxhound/`, only `goodfetch-protocol` and `xtask` name
/// the protocol's `foxhound` module (ADR 0099), in product code, tests and
/// benches alike.
#[rstest]
#[case::through_use(ENGINE_ROOT, "use goodfetch_protocol::foxhound::crawl::CRAWL_START;\n")]
#[case::inside_a_use_group(ENGINE_ROOT, "use goodfetch_protocol::{catalog::Product, foxhound};\n")]
#[case::through_a_qualified_path(
    ENGINE_ROOT,
    "pub fn name() -> &'static str {\n    goodfetch_protocol::foxhound::crawl::CRAWL_START.name\n}\n"
)]
#[case::inside_a_macro(
    ENGINE_ROOT,
    "pub fn log() {\n    tracing::info!(name = goodfetch_protocol::foxhound::crawl::CRAWL_START.name);\n}\n"
)]
#[case::under_the_crates_other_name(
    ENGINE_ROOT,
    "use goodfetch_protocol as protocol;\n\npub fn name() -> &'static str {\n    protocol::foxhound::crawl::CRAWL_START.name\n}\n"
)]
#[case::under_the_other_name_a_use_group_gives(
    ENGINE_ROOT,
    "use goodfetch_protocol::{self as protocol, catalog::Product};\n\npub fn name() -> &'static str {\n    protocol::foxhound::crawl::CRAWL_START.name\n}\n"
)]
#[case::through_a_glob_of_the_crates_root(ENGINE_ROOT, "use goodfetch_protocol::*;\n")]
#[case::through_a_glob_inside_a_use_group(
    ENGINE_ROOT,
    "use goodfetch_protocol::{catalog::Product, *};\n"
)]
#[case::in_a_test(
    "crates/goodfetch-engine/tests/it/main.rs",
    "use goodfetch_protocol::foxhound::settings::PROVENANCE_LOG;\n"
)]
#[case::in_a_bench(
    "crates/goodfetch-engine/benches/messages.rs",
    "use goodfetch_protocol::foxhound::crawl::CRAWL_START;\n"
)]
fn crate_outside_foxhound_naming_the_foxhound_module_fails(
    #[case] file: &str,
    #[case] source: &str,
) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.write(
        file,
        format!("//! Fixture engine crate.\n\n{source}").as_bytes(),
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("names-foxhound-module", file)]
    );
    assert_eq!(outcome.code, 1);
}

/// A name the crate's root gives the protocol's crate is a way to the module
/// from every file of the crate.
#[test]
fn crate_outside_foxhound_naming_the_foxhound_module_under_a_name_another_file_gives_fails() {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.write(
        ENGINE_ROOT,
        b"//! Fixture engine crate.\n\npub(crate) use goodfetch_protocol as protocol;\n\nmod other;\n",
    );
    workspace.write(
        "crates/goodfetch-engine/src/other.rs",
        b"//! Fixture module.\n\nuse crate::protocol::foxhound::crawl::CRAWL_START;\n",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "names-foxhound-module",
            "crates/goodfetch-engine/src/other.rs"
        )],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// A manifest may rename the dependency (`package = "goodfetch-protocol"`):
/// the name it gives is the crate's in that package's code.
#[test]
fn crate_outside_foxhound_naming_the_foxhound_module_under_its_manifests_name_fails() {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.write(
        "crates/goodfetch-engine/Cargo.toml",
        b"[package]\nname = \"goodfetch-engine\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\ngoodfetch-workspace-hack = { path = \"../goodfetch-workspace-hack\" }\nwire-types = { package = \"goodfetch-protocol\", path = \"../goodfetch-protocol\" }\n",
    );
    workspace.write(
        ENGINE_ROOT,
        b"//! Fixture engine crate.\n\nuse wire_types::foxhound::crawl::CRAWL_START;\n",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("names-foxhound-module", ENGINE_ROOT)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// The generated names of what only Foxhound carries are how a Collie
/// engine refuses them (ADR 0087): they are not the module.
#[test]
fn crate_outside_foxhound_naming_the_generated_names_passes() {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.write(
        ENGINE_ROOT,
        b"//! Fixture engine crate.\n\nuse goodfetch_protocol::catalog::*;\nuse goodfetch_protocol::foxhound_names;\n\n/// Foxhound's, a word and no path.\npub fn foxhound() -> usize {\n    foxhound_names::OPERATIONS.len()\n}\n",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

/// `goodfetch-protocol` defines the module, `xtask` reads it for codegen and
/// Foxhound's crates are whom it is for.
#[rstest]
#[case::the_protocol("crates/goodfetch-protocol/tests/it/main.rs")]
#[case::xtask("xtask/src/lib.rs")]
#[case::a_crate_of_foxhound("crates/foxhound/goodfetch-crawl/src/lib.rs")]
fn crate_that_may_name_the_foxhound_module_passes(#[case] file: &str) {
    let workspace = Workspace::from_fixture("dependency-direction");
    workspace.write(
        "Cargo.toml",
        b"[workspace]\nresolver = \"3\"\nmembers = [\"crates/goodfetch-*\", \"crates/foxhound/goodfetch-*\", \"proving-ground\", \"xtask\"]\n",
    );
    workspace.write(
        "crates/foxhound/goodfetch-crawl/Cargo.toml",
        b"[package]\nname = \"goodfetch-crawl\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\ngoodfetch-workspace-hack = { path = \"../../goodfetch-workspace-hack\" }\n",
    );
    workspace.write(
        "crates/foxhound/goodfetch-crawl/src/lib.rs",
        b"//! Fixture crate of Foxhound.\n",
    );
    workspace.write(
        file,
        b"//! Fixture.\n\nuse goodfetch_protocol::foxhound::crawl::CRAWL_START;\n",
    );

    let outcome = workspace.xtask(&["dependency-direction"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}
