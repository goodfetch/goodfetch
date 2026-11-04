//! The selection rules, as named cases of changed paths over one fixture
//! workspace that holds both Products' crates, the sink processes, both
//! Products' TypeScript and Python packages, and the Console's visual test
//! (spec #130, Testing Decisions).
//! Covers: story:1-52, story:1-53, story:1-64

use std::collections::BTreeSet;

use rstest::rstest;

use super::selection::{Job, Selection, select};
use super::workspace::{Crate, Ecosystem, Embedded, Package, Workspace};
use super::{RepoPath, WorkspaceMembers, select_with_readers};

fn names(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn library(name: &str, folder: &str, dependencies: &[&str]) -> Crate {
    Crate {
        name: name.to_owned(),
        folder: RepoPath::new(folder),
        dependencies: names(dependencies),
        built_from: names(dependencies),
        benches: BTreeSet::new(),
        executable: false,
        runs: BTreeSet::new(),
    }
}

fn executable(name: &str, folder: &str, dependencies: &[&str]) -> Crate {
    Crate {
        executable: true,
        ..library(name, folder, dependencies)
    }
}

fn with_benches(benches: &[&str], krate: Crate) -> Crate {
    Crate {
        benches: names(benches),
        ..krate
    }
}

/// A crate whose tests also depend on `dev`, which its build does not.
fn testing_with(dev: &[&str], krate: Crate) -> Crate {
    let mut dependencies = krate.dependencies.clone();
    dependencies.extend(names(dev));
    Crate {
        dependencies,
        ..krate
    }
}

/// A crate whose tests run the executables of `runs`.
fn running(runs: &[&str], krate: Crate) -> Crate {
    Crate {
        runs: names(runs),
        ..krate
    }
}

fn package(ecosystem: Ecosystem, name: &str, folder: &str, dependencies: &[&str]) -> Package {
    Package {
        ecosystem,
        name: name.to_owned(),
        folder: RepoPath::new(folder),
        dependencies: names(dependencies),
        runtime_dependencies: names(dependencies),
        runs: BTreeSet::new(),
        loaded_by_every_run: false,
    }
}

/// A package whose tests run the executables of `runs`.
fn package_running(runs: &[&str], package: Package) -> Package {
    Package {
        runs: names(runs),
        ..package
    }
}

/// Collie's crates outside `crates/foxhound/`, Foxhound's inside it, the sink
/// processes and their shared code under `crates/foxhound/sinks/` (the sink
/// protocol is `goodfetch-protocol`'s), each Product's packages, and a visual
/// test in the Console, which takes its screenshots through the shared
/// test-support package. Test support is a dev-dependency only, and the
/// conformance crate's and test-support packages' tests run the Proving
/// ground's and the Products' executables, as in the repository.
fn fixture() -> Workspace {
    Workspace {
        crates: crates(),
        packages: packages(),
        visual_tests: vec![RepoPath::new("console/src/screens/run.visual.test.tsx")],
        models: Vec::new(),
        embedded: Vec::new(),
    }
}

fn crates() -> Vec<Crate> {
    vec![
        library("goodfetch-protocol", "crates/goodfetch-protocol", &[]),
        library(
            "goodfetch-test-support",
            "crates/goodfetch-test-support",
            &[],
        ),
        with_benches(
            &["engine"],
            testing_with(
                &["goodfetch-test-support"],
                library(
                    "goodfetch-engine",
                    "crates/goodfetch-engine",
                    &["goodfetch-protocol"],
                ),
            ),
        ),
        executable(
            "goodfetch-collie",
            "crates/goodfetch-collie",
            &["goodfetch-engine"],
        ),
        with_benches(
            &["frontier"],
            library(
                "goodfetch-frontier",
                "crates/foxhound/goodfetch-frontier",
                &["goodfetch-engine"],
            ),
        ),
        executable(
            "goodfetch-foxhound",
            "crates/foxhound/goodfetch-foxhound",
            &["goodfetch-engine", "goodfetch-frontier"],
        ),
        library(
            "goodfetch-sink-shared",
            "crates/foxhound/sinks/goodfetch-sink-shared",
            &["goodfetch-protocol"],
        ),
        executable(
            "goodfetch-sink-kafka",
            "crates/foxhound/sinks/goodfetch-sink-kafka",
            &["goodfetch-sink-shared"],
        ),
        executable(
            "goodfetch-sink-s3",
            "crates/foxhound/sinks/goodfetch-sink-s3",
            &["goodfetch-sink-shared"],
        ),
        testing_with(
            &["goodfetch-test-support"],
            executable("goodfetch-proving-ground", "proving-ground", &[]),
        ),
        running(
            &[
                "goodfetch-proving-ground",
                "goodfetch-collie",
                "goodfetch-foxhound",
            ],
            testing_with(
                &["goodfetch-test-support"],
                library(
                    "goodfetch-conformance",
                    "crates/goodfetch-conformance",
                    &["goodfetch-protocol"],
                ),
            ),
        ),
        executable("xtask", "xtask", &[]),
    ]
}

fn packages() -> Vec<Package> {
    use Ecosystem::{Python, TypeScript};
    vec![
        package(
            TypeScript,
            "@goodfetch/collie-client",
            "sdks/typescript/collie-client",
            &[],
        ),
        package(
            TypeScript,
            "@goodfetch/collie",
            "sdks/typescript/collie",
            &["@goodfetch/collie-client"],
        ),
        package(
            TypeScript,
            "@goodfetch/foxhound",
            "sdks/typescript/foxhound",
            &["@goodfetch/collie-client"],
        ),
        package(
            TypeScript,
            "@goodfetch/console",
            "console",
            &["@goodfetch/test-support"],
        ),
        package(
            TypeScript,
            "@goodfetch/console-foxhound",
            "console/foxhound",
            &[],
        ),
        package_running(
            &["goodfetch-collie", "goodfetch-proving-ground"],
            package(
                TypeScript,
                "@goodfetch/test-support",
                "test-support/typescript",
                &[],
            ),
        ),
        package(
            Python,
            "goodfetch-collie-client",
            "sdks/python/collie-client",
            &[],
        ),
        package(
            Python,
            "goodfetch-collie",
            "sdks/python/collie",
            &["goodfetch-collie-client"],
        ),
        package(
            Python,
            "goodfetch-foxhound",
            "sdks/python/foxhound",
            &["goodfetch-collie-client"],
        ),
        package_running(
            &["goodfetch-proving-ground"],
            package(Python, "goodfetch-test-support", "test-support/python", &[]),
        ),
    ]
}

fn select_paths(paths: &[&str]) -> Selection {
    let paths: Vec<RepoPath> = paths.iter().map(|path| RepoPath::new(*path)).collect();
    select(&fixture(), &paths)
}

/// The value of the selection's `key=value` line for `key`.
fn value(selection: &Selection, key: &str) -> String {
    let text = selection.to_string();
    let prefix = format!("{key}=");
    let line = text
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no `{key}` line in:\n{text}"));
    line[prefix.len()..].to_owned()
}

#[rstest]
// The lab's own workflow runs on `rnd/` (ADR 0032); the tier selects nothing.
#[case::nothing_for_rnd(&["rnd/lab/src/cli.ts"], "[]")]
#[case::docs_jobs_only_for_docs(&["docs/collie/index.md", "website/index.html"], r#"["docs"]"#)]
#[case::nothing_beyond_quick_for_top_level_markdown(&["GLOSSARY.md"], "[]")]
#[case::pre_merge_and_sinks_for_a_crate(&["crates/foxhound/sinks/goodfetch-sink-s3/src/main.rs"], r#"["pre-merge","sinks"]"#)]
#[case::pre_merge_and_sinks_for_a_package(&["sdks/python/collie/src/goodfetch_collie/client.py"], r#"["pre-merge","sinks"]"#)]
#[case::perf_for_a_crate_with_bench_targets(&["crates/foxhound/goodfetch-frontier/src/lib.rs"], r#"["pre-merge","sinks","perf"]"#)]
#[case::evaluation_for_prompts(&["crates/goodfetch-engine/src/agent/prompts/act.md"], r#"["pre-merge","sinks","perf","visual","evaluation"]"#)]
#[case::evaluation_for_request_snapshots(&["crates/goodfetch-collie/tests/it/request-snapshots/act.snap"], r#"["pre-merge","sinks","visual","evaluation"]"#)]
// The catalog is the docs' reference too (spec #130, Pre-merge suite).
// The Kennel is built on the protocol crate alone (ADR 0141).
#[case::evaluation_docs_and_kennel_for_catalog_descriptions(&["crates/goodfetch-protocol/src/catalog/act.rs"], r#"["pre-merge","sinks","perf","visual","docs","evaluation","kennel"]"#)]
// The Kennel's own workspace selects its checks and nothing else (ADR 0141).
#[case::only_kennel_for_the_kennels_code(&["kennel/crates/goodfetch-kennel/src/state.rs"], r#"["kennel"]"#)]
#[case::only_kennel_for_the_kennels_lockfile(&["kennel/Cargo.lock", "kennel/deny.toml"], r#"["kennel"]"#)]
#[case::evaluation_for_skills(&["skills/collie/SKILL.md"], r#"["pre-merge","sinks","perf","visual","docs","evaluation"]"#)]
#[case::every_job_but_evaluation_for_workspace_inputs(&["Cargo.lock"], r#"["pre-merge","sinks","perf","visual","docs"]"#)]
// The visual job runs when a visual test, a Reference screenshot, a runner's
// config, the pinned image or the code a visual test exercises changes.
#[case::visual_for_a_vitest_visual_test(&["console/src/screens/run.visual.test.tsx"], r#"["pre-merge","sinks","visual"]"#)]
#[case::visual_for_a_playwright_visual_test(&["test-support/typescript/src/screenshot.visual.spec.ts"], r#"["pre-merge","sinks","visual"]"#)]
#[case::visual_for_a_vitest_reference_screenshot(&["console/src/screens/__screenshots__/run.visual.test.tsx/run-1.png"], r#"["pre-merge","sinks","visual"]"#)]
#[case::visual_for_a_playwright_reference_screenshot(&["test-support/typescript/src/screenshot.visual.spec.ts-snapshots/fixture-page.png"], r#"["pre-merge","sinks","visual"]"#)]
#[case::visual_for_the_playwright_config(&["playwright.config.ts"], r#"["pre-merge","sinks","perf","visual","docs"]"#)]
#[case::visual_for_the_pinned_image(&["ci/visual/image.toml"], r#"["pre-merge","sinks","perf","visual","docs"]"#)]
#[case::visual_for_code_a_visual_test_exercises(&["console/src/app.tsx"], r#"["pre-merge","sinks","visual"]"#)]
#[case::visual_for_a_package_a_visual_test_depends_on(&["test-support/typescript/src/screenshot.ts"], r#"["pre-merge","sinks","visual"]"#)]
#[case::no_visual_for_a_package_no_visual_test_reaches(&["sdks/typescript/collie/src/index.ts"], r#"["pre-merge","sinks"]"#)]
#[case::no_visual_for_a_nested_package_without_visual_tests(&["console/foxhound/src/datasets.tsx"], r#"["pre-merge","sinks"]"#)]
// Test support's tests run Collie's executable, and the Console with its
// visual test depends on test support.
#[case::visual_for_an_executable_a_visual_tests_package_runs(&["crates/goodfetch-collie/src/main.rs"], r#"["pre-merge","sinks","visual"]"#)]
#[case::nothing_for_no_change(&[], "[]")]
fn changed_paths_select_their_jobs(#[case] paths: &[&str], #[case] jobs: &str) {
    let selection = select_paths(paths);

    assert_eq!(value(&selection, "jobs"), jobs);
}

// The local gate runs its quick form only when no test, benchmark, model,
// container or Evaluation reads the change (ADR 0135).
#[rstest]
#[case::top_level_markdown(&["GLOSSARY.md", "AGENTS.md"], "quick")]
#[case::an_adr(&["docs/adr/0135-local-gate.md"], "quick")]
#[case::docs_and_website(&["docs/collie/index.md", "website/index.html"], "quick")]
#[case::the_lab(&["rnd/lab/src/cli.ts"], "quick")]
#[case::a_crate(&["crates/goodfetch-collie/src/main.rs"], "full")]
#[case::markdown_a_crate_owns(&["crates/foxhound/README.md"], "full")]
#[case::a_skill(&["skills/collie/SKILL.md"], "full")]
#[case::a_prompt(&["crates/goodfetch-engine/src/agent/prompts/act.md"], "full")]
#[case::a_workspace_input(&["Cargo.lock"], "full")]
#[case::docs_with_a_crate(&["docs/adr/0135-local-gate.md", "crates/goodfetch-collie/src/main.rs"], "full")]
#[case::a_visual_input(&["ci/visual/image.toml"], "full")]
#[case::the_kennel(&["kennel/README.md"], "full")]
fn the_gate_is_quick_only_when_no_job_but_the_docs_runs(
    #[case] paths: &[&str],
    #[case] gate: &str,
) {
    let selection = select_paths(paths);

    assert_eq!(value(&selection, "gate"), gate);
}

/// Writes `text` to `path` under `root`, its folders first.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]
fn write(root: &std::path::Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A test that reads a file no member owns names it in a string literal: a
/// change to that file alone selects the test's member and the full gate
/// (finding 586-9). A document the Quick suite alone checks (top-level
/// Markdown, `docs/`, `rnd/`, `website/`) is read only by a literal naming
/// the file itself: a literal naming one of its folders is a fixture path or
/// a word (`"docs" =>`), and xtask's checks that walk those folders run in
/// `just lint`, which the quick gate runs. A comment or a possessive names a
/// file without reading it (#2041).
#[rstest]
#[case::glossary_and_adrs_beside_a_folder_named_by_a_fixture(
    r#"workspace.write("docs/adr/0135-local-gate.md", "");"#,
    &["GLOSSARY.md", "docs/adr/0086-politeness.md", "docs/adr/0138-analysis.md"],
    "quick",
    false
)]
#[case::an_adr_beside_its_folder_as_a_word(
    r#"match segment { "docs" => 1, _ => 0 }"#,
    &["docs/adr/0086-politeness.md"],
    "quick",
    false
)]
#[case::research_beside_its_folder_as_a_word(
    r#"const SKIPPED: [&str; 1] = ["rnd"];"#,
    &["rnd/lab/src/cli.ts"],
    "quick",
    false
)]
#[case::a_comment_naming_the_file(
    "/// `TESTING.md`'s path traversal cases.\n//! See TESTING.md\".\n",
    &["GLOSSARY.md"],
    "quick",
    false
)]
#[case::a_possessive_in_a_string(
    r#"#[expect(unused, reason = "GLOSSARY.md's terms")]"#,
    &["GLOSSARY.md"],
    "quick",
    false
)]
#[case::an_adr_the_code_reads(
    r#"const MATRIX: &str = "docs/adr/0037-v1-scope-matrix.md";"#,
    &["GLOSSARY.md", "docs/adr/0037-v1-scope-matrix.md"],
    "full",
    true
)]
#[case::a_document_a_test_reads(
    r#"let notes = include_str!("../../../NOTES.md"); // its rows"#,
    &["NOTES.md"],
    "full",
    true
)]
fn a_document_is_read_only_where_code_names_the_file_itself(
    #[case] source: &str,
    #[case] paths: &[&str],
    #[case] gate: &str,
    #[case] selected: bool,
) {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "crates/goodfetch-collie/tests/it.rs", source);
    let members = WorkspaceMembers::from_workspace(fixture());
    let paths: Vec<RepoPath> = paths.iter().map(|path| RepoPath::new(*path)).collect();

    let selection = select_with_readers(root.path(), &members, &paths).unwrap();

    assert_eq!(
        (
            value(&selection, "gate"),
            value(&selection, "crates")
                .split(' ')
                .any(|krate| krate == "goodfetch-collie"),
        ),
        (gate.to_owned(), selected)
    );
}

/// xtask's commands run at the repository's root, so its literals naming a
/// document there are its checks' inputs, which `just lint` and the floor
/// guard run in the quick gate, or its fixture workspaces' files; its tests
/// reach the repository's own documents from their folder, through `../`.
#[rstest]
#[case::a_fixture_file(r##"workspace.write("NOTES.md", b"# Notes\n");"##, "quick")]
#[case::a_test_reading_the_repository_document(
    r#"const NOTES: &str = include_str!("../../../NOTES.md");"#,
    "full"
)]
fn xtask_reads_a_document_only_through_a_path_from_its_folder(
    #[case] source: &str,
    #[case] gate: &str,
) {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "xtask/tests/it/notes.rs", source);
    let members = WorkspaceMembers::from_workspace(fixture());

    let selection =
        select_with_readers(root.path(), &members, &[RepoPath::new("NOTES.md")]).unwrap();

    assert_eq!(value(&selection, "gate"), gate);
}

/// `xtask counts`, in the perf job, holds the counted rows to the limits
/// of `TESTING.md`'s Quality register, so a change to it runs perf whatever
/// test reads it; the register check, in `just lint`, runs in either gate.
#[test]
fn a_change_to_the_register_runs_perf() {
    let selection = select_paths(&["TESTING.md"]);

    assert!(selection.selects(Job::Perf), "{selection}");
}

#[rstest]
#[case::sink_protocol_reaches_every_sink(&["crates/goodfetch-protocol/src/sink.rs"], "goodfetch-sink-kafka goodfetch-sink-s3")]
#[case::shared_sink_code_reaches_every_sink(&["crates/foxhound/sinks/goodfetch-sink-shared/src/lib.rs"], "goodfetch-sink-kafka goodfetch-sink-s3")]
#[case::one_sink_reaches_itself(&["crates/foxhound/sinks/goodfetch-sink-s3/src/main.rs"], "goodfetch-sink-s3")]
#[case::other_crate_reaches_no_sink(&["crates/goodfetch-collie/src/main.rs"], "")]
fn changed_sinks_are_the_sinks_a_change_reaches(#[case] paths: &[&str], #[case] sinks: &str) {
    let selection = select_paths(paths);

    assert_eq!(value(&selection, "sinks-touched"), sinks);
}

#[rstest]
#[case::one_sink(&["crates/foxhound/sinks/goodfetch-sink-s3/src/main.rs"])]
#[case::no_sink(&["crates/goodfetch-collie/src/main.rs"])]
fn every_sink_runs_until_the_merge_queue_exists(#[case] paths: &[&str]) {
    let selection = select_paths(paths);

    assert_eq!(
        value(&selection, "sinks"),
        "goodfetch-sink-kafka goodfetch-sink-s3"
    );
}

#[test]
fn no_sink_runs_without_the_pre_merge_suite() {
    let selection = select_paths(&["docs/collie/index.md"]);

    assert_eq!(value(&selection, "sinks"), "");
}

#[rstest]
#[case::foxhound_crate(&["crates/foxhound/goodfetch-frontier/src/lib.rs"], "foxhound")]
#[case::sink_process(&["crates/foxhound/sinks/goodfetch-sink-s3/src/main.rs"], "foxhound")]
#[case::foxhound_console_package(&["console/foxhound/src/datasets.tsx"], "foxhound")]
#[case::foxhound_sdk_files(&["sdks/typescript/foxhound/src/index.ts", "sdks/python/foxhound/src/goodfetch_foxhound/__init__.py"], "foxhound")]
#[case::foxhound_and_shared_crate(&["crates/foxhound/goodfetch-frontier/src/lib.rs", "crates/goodfetch-engine/src/lib.rs"], "collie foxhound")]
#[case::collie_sdk_files(&["sdks/typescript/collie/src/index.ts"], "collie foxhound")]
#[case::docs(&["docs/foxhound/index.md"], "collie foxhound")]
fn change_confined_to_foxhounds_side_selects_only_foxhound(
    #[case] paths: &[&str],
    #[case] products: &str,
) {
    let selection = select_paths(paths);

    assert_eq!(value(&selection, "products"), products);
}

#[rstest]
#[case::crate_and_its_dependants(&["crates/goodfetch-engine/src/lib.rs"], "goodfetch-engine/engine goodfetch-frontier/frontier")]
#[case::crate_without_dependants(&["crates/foxhound/goodfetch-frontier/benches/frontier.rs"], "goodfetch-frontier/frontier")]
#[case::crate_whose_dependants_have_none(&["crates/foxhound/sinks/goodfetch-sink-shared/src/lib.rs"], "")]
// A ceiling changed in `xtask perf`'s budget is checked at once on the bench
// targets of the crates it reads (ADR 0134).
#[case::ceilings_of_the_performance_budget(&["xtask/src/perf/budget.rs"], "goodfetch-frontier/frontier")]
// A newly pinned reference build is compared at once with every bench target
// (`drift.reference`, ADR 0140).
#[case::the_pinned_reference_build(&["ci/perf-reference.toml"], "goodfetch-engine/engine goodfetch-frontier/frontier")]
fn changed_crate_selects_its_and_its_dependants_bench_targets(
    #[case] paths: &[&str],
    #[case] bench_targets: &str,
) {
    let selection = select_paths(paths);

    assert_eq!(value(&selection, "bench-targets"), bench_targets);
}

#[rstest]
#[case::changed_crates(&["crates/foxhound/sinks/goodfetch-sink-shared/src/lib.rs", "xtask/src/main.rs"], "rdeps(=goodfetch-sink-shared) | rdeps(=xtask)")]
#[case::workspace_input(&["Cargo.toml"], "all()")]
#[case::no_crate(&["sdks/typescript/collie/src/index.ts"], "none()")]
// The Proving ground's own integration tests (`goodfetch-proving-ground::it`,
// such as its task list's) are among `rdeps(=goodfetch-proving-ground)`'s.
#[case::executable_and_the_crates_whose_tests_run_it(&["proving-ground/src/oracle.rs"], "rdeps(=goodfetch-conformance) | rdeps(=goodfetch-proving-ground)")]
#[case::what_an_executable_is_built_from_and_the_crates_whose_tests_run_it(&["crates/goodfetch-engine/src/lib.rs"], "rdeps(=goodfetch-conformance) | rdeps(=goodfetch-engine)")]
fn changed_crates_select_their_dependants_through_rdeps(
    #[case] paths: &[&str],
    #[case] filter: &str,
) {
    let selection = select_paths(paths);

    assert_eq!(value(&selection, "nextest-filter"), filter);
}

#[rstest]
#[case::crate_and_its_dependants(&["crates/foxhound/sinks/goodfetch-sink-shared/src/lib.rs"], "goodfetch-sink-kafka goodfetch-sink-s3 goodfetch-sink-shared")]
#[case::dev_dependency_reaches_its_dependants(&["crates/goodfetch-test-support/src/lib.rs"], "goodfetch-collie goodfetch-conformance goodfetch-engine goodfetch-foxhound goodfetch-frontier goodfetch-proving-ground goodfetch-test-support")]
#[case::no_crate(&["sdks/typescript/collie/src/index.ts"], "")]
// Its tests run the Proving ground's executable through the test tool manifest.
#[case::executable_reaches_the_crates_whose_tests_run_it(&["proving-ground/src/oracle.rs"], "goodfetch-conformance goodfetch-proving-ground")]
// The Products' executables are built from it, and the conformance crate's
// tests run them.
#[case::what_an_executable_is_built_from_reaches_the_crates_whose_tests_run_it(&["crates/goodfetch-engine/src/lib.rs"], "goodfetch-collie goodfetch-conformance goodfetch-engine goodfetch-foxhound goodfetch-frontier")]
// Only Foxhound's executable is built from it, and no Cargo dependency
// points from the conformance crate to it: the conformance crate's tests
// still run, as they drive that executable (#950's `goodfetch-fetch-tier`).
#[case::what_only_foxhounds_executable_is_built_from_reaches_the_crates_whose_tests_run_it(&["crates/foxhound/goodfetch-frontier/src/lib.rs"], "goodfetch-conformance goodfetch-foxhound goodfetch-frontier")]
fn changed_crates_list_them_and_their_dependants_for_cargo(
    #[case] paths: &[&str],
    #[case] crates: &str,
) {
    let selection = select_paths(paths);

    assert_eq!(value(&selection, "crates"), crates);
}

#[rstest]
#[case::client_package_and_its_dependants(&["sdks/typescript/collie-client/src/index.ts"], "sdks/typescript/collie sdks/typescript/collie-client sdks/typescript/foxhound", "")]
#[case::python_package(&["sdks/python/collie/src/goodfetch_collie/client.py"], "", "sdks/python/collie")]
#[case::nested_package_owns_its_files(&["console/foxhound/src/datasets.tsx"], "console/foxhound", "")]
// The test-support packages' tests run these executables through the test
// tool manifest, and the Console's run them through test support.
#[case::executable_reaches_the_packages_whose_tests_run_it(&["proving-ground/src/oracle.rs"], "console test-support/typescript", "test-support/python")]
#[case::what_an_executable_is_built_from_reaches_the_packages_whose_tests_run_it(&["crates/goodfetch-engine/src/lib.rs"], "console test-support/typescript", "")]
// Test support is no part of the Products' builds: its change changes no
// executable a package runs.
#[case::a_dev_dependency_reaches_no_package_through_an_executable(&["crates/goodfetch-test-support/src/lib.rs"], "", "")]
fn changed_packages_select_themselves_and_their_dependants(
    #[case] paths: &[&str],
    #[case] vitest: &str,
    #[case] pytest: &str,
) {
    let selection = select_paths(paths);

    assert_eq!(
        (
            value(&selection, "vitest-packages"),
            value(&selection, "pytest-packages")
        ),
        (vitest.to_owned(), pytest.to_owned())
    );
}

#[test]
fn shared_crate_change_selects_both_products_tests() {
    let selection = select_paths(&["crates/goodfetch-engine/src/lib.rs"]);

    insta::assert_snapshot!(selection, @r#"
    products=collie foxhound
    jobs=["pre-merge","sinks","perf","visual"]
    nextest-filter=rdeps(=goodfetch-conformance) | rdeps(=goodfetch-engine)
    crates=goodfetch-collie goodfetch-conformance goodfetch-engine goodfetch-foxhound goodfetch-frontier
    vitest-packages=console test-support/typescript
    pytest-packages=
    sinks=goodfetch-sink-kafka goodfetch-sink-s3
    sinks-touched=
    bench-targets=goodfetch-engine/engine goodfetch-frontier/frontier
    models=
    gate=full
    "#);
}

#[test]
fn foxhound_side_change_outside_any_package_selects_all_of_foxhounds_tests() {
    let selection = select_paths(&["crates/foxhound/README.md"]);

    insta::assert_snapshot!(selection, @r#"
    products=foxhound
    jobs=["pre-merge","sinks","perf","docs"]
    nextest-filter=rdeps(=goodfetch-conformance) | rdeps(=goodfetch-foxhound) | rdeps(=goodfetch-frontier) | rdeps(=goodfetch-sink-kafka) | rdeps(=goodfetch-sink-s3) | rdeps(=goodfetch-sink-shared)
    crates=goodfetch-conformance goodfetch-foxhound goodfetch-frontier goodfetch-sink-kafka goodfetch-sink-s3 goodfetch-sink-shared
    vitest-packages=console/foxhound sdks/typescript/foxhound
    pytest-packages=sdks/python/foxhound
    sinks=goodfetch-sink-kafka goodfetch-sink-s3
    sinks-touched=goodfetch-sink-kafka goodfetch-sink-s3
    bench-targets=goodfetch-frontier/frontier
    models=
    gate=full
    "#);
}

#[test]
fn workspace_input_change_selects_everything() {
    let selection = select_paths(&["justfile"]);

    insta::assert_snapshot!(selection, @r#"
    products=collie foxhound
    jobs=["pre-merge","sinks","perf","visual","docs"]
    nextest-filter=all()
    crates=goodfetch-collie goodfetch-conformance goodfetch-engine goodfetch-foxhound goodfetch-frontier goodfetch-protocol goodfetch-proving-ground goodfetch-sink-kafka goodfetch-sink-s3 goodfetch-sink-shared goodfetch-test-support xtask
    vitest-packages=console console/foxhound sdks/typescript/collie sdks/typescript/collie-client sdks/typescript/foxhound test-support/typescript
    pytest-packages=sdks/python/collie sdks/python/collie-client sdks/python/foxhound test-support/python
    sinks=goodfetch-sink-kafka goodfetch-sink-s3
    sinks-touched=goodfetch-sink-kafka goodfetch-sink-s3
    bench-targets=goodfetch-engine/engine goodfetch-frontier/frontier
    models=
    gate=full
    "#);
}

/// The fixture with the test-support packages loaded into every run of
/// their runners: the root Vitest and Playwright configs import the
/// TypeScript one, and the Python one is a pytest plugin.
fn with_test_support_in_every_run() -> Workspace {
    let mut workspace = fixture();
    for package in &mut workspace.packages {
        if package.folder.is_in("test-support") {
            package.loaded_by_every_run = true;
        }
    }
    workspace
}

#[rstest]
#[case::typescript(
    "test-support/typescript/src/errors.ts",
    "vitest-packages",
    "console console/foxhound sdks/typescript/collie sdks/typescript/collie-client sdks/typescript/foxhound test-support/typescript"
)]
#[case::python(
    "test-support/python/src/goodfetch_test_support/plugin.py",
    "pytest-packages",
    "sdks/python/collie sdks/python/collie-client sdks/python/foxhound test-support/python"
)]
fn a_package_every_run_of_its_runner_loads_reaches_every_package_of_its_ecosystem(
    #[case] path: &str,
    #[case] key: &str,
    #[case] expected: &str,
) {
    let selection = select(&with_test_support_in_every_run(), &[RepoPath::new(path)]);

    assert_eq!(value(&selection, key), expected);
}

/// Collie's executable embeds the Console's build (ADR 0057): a change to
/// the Console changes the executable, and so reaches what runs it.
#[test]
fn a_console_change_reaches_the_crate_that_embeds_its_build_and_what_runs_it() {
    let mut workspace = fixture();
    workspace.embedded = vec![Embedded {
        package: RepoPath::new("console"),
        krate: "goodfetch-collie".to_owned(),
    }];

    let selection = select(&workspace, &[RepoPath::new("console/src/app.tsx")]);

    assert_eq!(
        value(&selection, "crates"),
        "goodfetch-collie goodfetch-conformance"
    );
    assert_eq!(
        value(&selection, "vitest-packages"),
        "console test-support/typescript"
    );
}

/// Each Product's executable embeds its own Console build (ADR 0087):
/// Collie's crate Collie's package's, Foxhound's crate Foxhound's.
#[test]
fn each_products_executable_embeds_its_own_consoles_build() {
    let crates = vec![
        executable("goodfetch-collie", "crates/goodfetch-collie", &[]),
        executable(
            "goodfetch-foxhound",
            "crates/foxhound/goodfetch-foxhound",
            &[],
        ),
    ];
    let packages = vec![
        package(
            Ecosystem::TypeScript,
            "@goodfetch/console",
            "console/collie",
            &[],
        ),
        package(
            Ecosystem::TypeScript,
            "@goodfetch/console-foxhound",
            "console/foxhound",
            &["@goodfetch/console"],
        ),
    ];

    let embedded = super::workspace::embedded(&crates, &packages);

    let pairs: Vec<(&str, &str)> = embedded
        .iter()
        .map(|embedded| (embedded.package.as_str(), embedded.krate.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("console/collie", "goodfetch-collie"),
            ("console/foxhound", "goodfetch-foxhound"),
        ]
    );
}

/// A change to Collie's Console reaches Foxhound's executable too, whose
/// build composes Collie's screens with its own.
#[test]
fn a_change_to_collies_console_reaches_foxhounds_executable() {
    let mut workspace = fixture();
    for package in &mut workspace.packages {
        if package.folder.as_str() == "console/foxhound" {
            package.dependencies = names(&["@goodfetch/console"]);
            package.runtime_dependencies = names(&["@goodfetch/console"]);
        }
    }
    workspace.embedded = vec![
        Embedded {
            package: RepoPath::new("console"),
            krate: "goodfetch-collie".to_owned(),
        },
        Embedded {
            package: RepoPath::new("console/foxhound"),
            krate: "goodfetch-foxhound".to_owned(),
        },
    ];

    let selection = select(&workspace, &[RepoPath::new("console/src/app.tsx")]);

    let crates = value(&selection, "crates");
    assert!(crates.contains("goodfetch-collie"), "{selection}");
    assert!(crates.contains("goodfetch-foxhound"), "{selection}");
}

/// What the Console's tests alone use, here test support, is no part of
/// its build, so it does not reach the crate that embeds the build.
#[test]
fn a_package_the_console_needs_only_for_its_tests_does_not_reach_the_embedder() {
    let mut workspace = fixture();
    workspace.embedded = vec![Embedded {
        package: RepoPath::new("console"),
        krate: "goodfetch-collie".to_owned(),
    }];
    for package in &mut workspace.packages {
        if package.folder.as_str() == "console" {
            package.runtime_dependencies.clear();
        }
    }

    let selection = select(
        &workspace,
        &[RepoPath::new("test-support/typescript/src/screenshot.ts")],
    );

    assert!(
        !value(&selection, "crates").contains("goodfetch-collie"),
        "{selection}"
    );
}

/// The members that own a visual test, which the visual job runs (the
/// incremental gate's re-review, N4).
#[test]
fn the_members_that_own_a_visual_test_are_known() {
    let members = super::WorkspaceMembers::from_workspace(fixture());

    assert!(members.owns_a_visual_test("console"));
    assert!(!members.owns_a_visual_test("test-support/typescript"));
}
