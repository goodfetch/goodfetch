//! Windows Smart App Control executable admission conformance tests (ADR 0039,
//! issue 873):
//! - on every OS, verifies that `goodfetch-toy-plugin` and a real interpreter
//!   are admitted, that Windows Smart App Control OS error codes (`4551`,
//!   `4556..=4559`, `4580..=4582`) on a toy Plugin, cached interpreter, Chrome
//!   for Testing, or Lightpanda become the named outcome
//!   `executable_blocked_by_smart_app_control` naming the file in a schema-valid
//!   `ConformanceDocument` (never assumed admitted, never an opaque failure),
//!   and that unrelated spawn failures fail the case;
//! - on Windows (Release, Windows), probes the real toy Plugin, a cached
//!   interpreter, the pinned Chrome for Testing build, and Lightpanda (naming
//!   `not_pinned_for_platform` when no Windows Lightpanda build is pinned), and
//!   keeps the conformance report naming whether Smart App Control admits each.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io;
use std::path::{Path, PathBuf};

use goodfetch_conformance::{
    ExecutableAdmission, SmartAppControlObservations, probe_executable,
    write_smart_app_control_report,
};
use goodfetch_protocol::executables::CODE_INTEGRITY_REFUSALS;
use goodfetch_protocol::result_document::{
    CaseOutcome, Outcome, ProvingGroundCopy, ResultDocument,
};
use rstest::rstest;

use crate::schema_violations;

const TOY_PLUGIN: &str = env!("CARGO_BIN_EXE_goodfetch-toy-plugin");

fn proving_ground_copy() -> ProvingGroundCopy {
    ProvingGroundCopy {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        seed: 873,
    }
}

/// Every Windows code-integrity refusal code (`4551`, `4556..=4559`,
/// `4580..=4582`) maps to the named outcome
/// `executable_blocked_by_smart_app_control` naming the file, while unrelated
/// spawn errors fail the check (ADR 0039, issue 873).
///
/// Covers: story:7-105, error:executable_blocked_by_smart_app_control
#[rstest]
#[case::policy_violation(4551)]
#[case::reputation_4556(4556)]
#[case::reputation_4557(4557)]
#[case::reputation_4558(4558)]
#[case::reputation_4559(4559)]
#[case::reputation_4580(4580)]
#[case::reputation_4581(4581)]
#[case::reputation_4582(4582)]
fn smart_app_control_refusal_codes_become_named_outcomes_in_conformance_report(
    #[case] os_error: i32,
) {
    assert!(CODE_INTEGRITY_REFUSALS.contains(&os_error));
    let blocked = |path: &str| {
        ExecutableAdmission::from_spawn_result(
            Path::new(path),
            Err(io::Error::from_raw_os_error(os_error)),
        )
    };
    let observations = SmartAppControlObservations {
        toy_plugin: blocked(r"C:\plugins\toy\goodfetch-toy-plugin.exe"),
        cached_interpreter: blocked(r"C:\goodfetch\runtimes\node-22\node.exe"),
        chrome_for_testing: blocked(r"C:\goodfetch\browsers\chrome-for-testing\chrome.exe"),
        lightpanda: blocked(r"C:\goodfetch\browsers\lightpanda\lightpanda.exe"),
    };
    assert_eq!(
        observations.toy_plugin.outcome_name(),
        "executable_blocked_by_smart_app_control"
    );

    let document = write_smart_app_control_report(&observations, proving_ground_copy()).unwrap();
    assert_eq!(
        schema_violations(&ResultDocument::Conformance(document.clone())),
        Vec::<String>::new()
    );
    assert!(!document.blocks());
    for capability in document.capabilities() {
        assert_eq!(capability.outcome(), Outcome::Proved);
        for case in capability.cases() {
            assert_eq!(case.outcome(), CaseOutcome::Passed);
            let entry = &case.facts().entries[0];
            assert!(
                entry.checks[0]
                    .name
                    .contains("executable_blocked_by_smart_app_control"),
                "{}",
                entry.checks[0].name
            );
            assert_eq!(
                entry.evidence.messages[0]["error_code"],
                "executable_blocked_by_smart_app_control"
            );
            assert_eq!(entry.evidence.messages[0]["retryable"], false);
            assert!(
                Path::new(entry.evidence.messages[0]["file"].as_str().unwrap())
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe")),
                "{}",
                entry.evidence.messages[0]
            );
        }
    }
}

/// An unrelated spawn failure (such as `ENOENT`) fails the conformance case
/// instead of being reported as Smart App Control (positive control, ADR 0039).
///
/// Covers: story:7-105
#[test]
fn an_unrelated_spawn_failure_fails_the_smart_app_control_conformance_report() {
    let missing = PathBuf::from("/nonexistent/goodfetch-missing-executable");
    let observations = SmartAppControlObservations {
        toy_plugin: probe_executable(Path::new(TOY_PLUGIN), &[]),
        cached_interpreter: ExecutableAdmission::from_spawn_result(
            &missing,
            Err(io::Error::from(io::ErrorKind::NotFound)),
        ),
        chrome_for_testing: ExecutableAdmission::Admitted {
            file: PathBuf::from("/browsers/chrome"),
        },
        lightpanda: ExecutableAdmission::NotPinnedForPlatform {
            name: "lightpanda".to_owned(),
        },
    };

    let document = write_smart_app_control_report(&observations, proving_ground_copy()).unwrap();
    assert_eq!(
        schema_violations(&ResultDocument::Conformance(document.clone())),
        Vec::<String>::new()
    );
    assert!(document.blocks());
    let plugins = document
        .capabilities()
        .iter()
        .find(|c| c.name() == "plugins.executables")
        .unwrap();
    assert_eq!(plugins.outcome(), Outcome::Failed);
    let browsers = document
        .capabilities()
        .iter()
        .find(|c| c.name() == "browsers.executables")
        .unwrap();
    assert_eq!(browsers.outcome(), Outcome::Proved);
}

/// On every OS, starting the real toy Plugin and keeping a conformance report
/// produces a schema-valid document naming each executable's admission
/// outcome (ADR 0039, issue 873).
///
/// Covers: story:7-105, error:executable_blocked_by_smart_app_control
#[test]
fn toy_plugin_and_interpreter_admission_report_is_schema_valid() {
    let toy = probe_executable(Path::new(TOY_PLUGIN), &[]);
    assert!(
        matches!(
            toy,
            ExecutableAdmission::Admitted { .. }
                | ExecutableAdmission::BlockedBySmartAppControl { .. }
        ),
        "unexpected toy Plugin admission outcome: {toy:?}"
    );
    let interpreter = find_cached_interpreter().map_or_else(
        || ExecutableAdmission::NotPinnedForPlatform {
            name: "interpreter".to_owned(),
        },
        |(path, args)| probe_executable(&path, args),
    );
    assert!(
        matches!(
            interpreter,
            ExecutableAdmission::Admitted { .. }
                | ExecutableAdmission::BlockedBySmartAppControl { .. }
                | ExecutableAdmission::NotPinnedForPlatform { .. }
        ),
        "unexpected interpreter admission outcome: {interpreter:?}"
    );

    let env = goodfetch_test_support::TestEnv::new();
    let chrome = find_pinned_chrome_for_testing(&env).map_or_else(
        || ExecutableAdmission::NotPinnedForPlatform {
            name: "chrome_for_testing".to_owned(),
        },
        |path| probe_executable(&path, &["--version"]),
    );
    let lightpanda = find_pinned_lightpanda().map_or_else(
        || ExecutableAdmission::NotPinnedForPlatform {
            name: "lightpanda".to_owned(),
        },
        |path| probe_executable(&path, &["--version"]),
    );

    let observations = SmartAppControlObservations {
        toy_plugin: toy,
        cached_interpreter: interpreter,
        chrome_for_testing: chrome,
        lightpanda,
    };
    let document = write_smart_app_control_report(&observations, proving_ground_copy()).unwrap();
    assert_eq!(
        schema_violations(&ResultDocument::Conformance(document.clone())),
        Vec::<String>::new()
    );
    assert!(!document.blocks(), "{document:#?}");
    crate::keep(&document, "smart-app-control");
}

fn find_cached_interpreter() -> Option<(PathBuf, &'static [&'static str])> {
    let candidates: &[(&str, &[&str])] = if cfg!(windows) {
        &[("node.exe", &["--version"]), ("python.exe", &["--version"])]
    } else {
        &[("node", &["--version"]), ("python3", &["--version"])]
    };
    let path_var = std::env::var_os("PATH")?;
    for (name, args) in candidates {
        for dir in std::env::split_paths(&path_var) {
            let full = dir.join(name);
            if full.is_file() {
                return Some((full, args));
            }
        }
    }
    None
}

fn find_pinned_chrome_for_testing(env: &goodfetch_test_support::TestEnv) -> Option<PathBuf> {
    let cache = goodfetch_test_support::browser_cache(env).ok()?;
    let exe_names: &[&str] = if cfg!(windows) {
        &["chrome-headless-shell.exe", "chrome.exe"]
    } else if cfg!(target_os = "macos") {
        &["chrome-headless-shell", "Google Chrome for Testing"]
    } else {
        &["chrome-headless-shell", "chrome"]
    };
    find_first_file(&cache, exe_names)
}

fn find_pinned_lightpanda() -> Option<PathBuf> {
    goodfetch_test_support::lightpanda_copy().ok()
}

fn find_first_file(root: &Path, names: &[&str]) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| names.contains(&name))
            {
                return Some(path);
            }
        }
    }
    None
}
