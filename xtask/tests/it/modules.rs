//! The module check (ADR 0126): each crate's `modules.toml` lists which of its
//! top-level modules may import which, and a file over 1,000 lines states why
//! it is still one concern.

use std::fmt::Write as _;

use rstest::rstest;

use crate::workspace::{Workspace, finding};

const CLOCK: &str = "crates/goodfetch-engine/src/clock.rs";
const MODULES_FILE: &str = "crates/goodfetch-engine/modules.toml";

#[test]
fn imports_the_file_allows_pass() {
    let workspace = Workspace::from_fixture("modules");

    let outcome = workspace.xtask(&["modules"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::through_use("clock-uses-retry.rs", CLOCK, "goodfetch-engine/clock->retry")]
#[case::through_a_qualified_path(
    "clock-names-retry-in-code.rs",
    CLOCK,
    "goodfetch-engine/clock->retry"
)]
#[case::inside_a_macro(
    "clock-names-retry-in-a-macro.rs",
    CLOCK,
    "goodfetch-engine/clock->retry"
)]
#[case::in_a_type_only(
    "clock-names-retry-in-a-signature.rs",
    CLOCK,
    "goodfetch-engine/clock->retry"
)]
#[case::from_a_submodule_through_super(
    "route-reaches-clock-through-super.rs",
    "crates/goodfetch-engine/src/dispatch/route.rs",
    "goodfetch-engine/dispatch->clock"
)]
#[case::inside_a_use_group(
    "dispatch-uses-a-group.rs",
    "crates/goodfetch-engine/src/dispatch.rs",
    "goodfetch-engine/dispatch->clock"
)]
fn import_the_file_does_not_allow_fails(
    #[case] case: &str,
    #[case] file: &str,
    #[case] subject: &str,
) {
    let workspace = Workspace::from_fixture("modules");
    workspace.add_fixture_file(&format!("modules-cases/{case}"), file);

    let outcome = workspace.xtask(&["modules"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("forbidden-import", subject)]
    );
    assert_eq!(outcome.code, 1);
}

/// A module whose `#[path]` names a file outside the usual layout, such as
/// another crate's source compiled in, is checked from that file, found
/// relative to the declaring file's folder as `rustc` finds it.
#[test]
fn module_from_a_path_attribute_is_checked_from_its_file() {
    let workspace = Workspace::from_fixture("modules");
    workspace.write(
        "crates/goodfetch-engine/src/dispatch.rs",
        b"//! Dispatch.\n\n#[path = \"../shared/route.rs\"]\nmod route;\n\npub fn dispatch() -> u64 {\n    route::route()\n}\n",
    );
    workspace.add_fixture_file(
        "modules-cases/route-reaches-clock-through-super.rs",
        "crates/goodfetch-engine/shared/route.rs",
    );

    let outcome = workspace.xtask(&["modules"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "forbidden-import",
            "goodfetch-engine/dispatch->clock"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::module_missing_from_the_file(
    "unlisted-module.toml",
    "module-not-listed",
    "goodfetch-engine/dispatch"
)]
#[case::name_that_is_no_module("unknown-module.toml", "unknown-module", "goodfetch-engine/cache")]
#[case::cycle_among_allowed_imports("cycle.toml", "import-cycle", "goodfetch-engine")]
fn modules_file_that_does_not_match_the_crate_fails(
    #[case] case: &str,
    #[case] kind: &str,
    #[case] subject: &str,
) {
    let workspace = Workspace::from_fixture("modules");
    workspace.add_fixture_file(&format!("modules-cases/{case}"), MODULES_FILE);

    let outcome = workspace.xtask(&["modules"]);

    assert_eq!(outcome.findings(), vec![finding(kind, subject)]);
    assert_eq!(outcome.code, 1);
}

/// A module's `[submodules.<module>]` table holds its own submodules to the
/// same rules (ADR 0126).
#[rstest]
#[case::through_a_crate_path("policy-uses-backoff.rs")]
#[case::through_super("policy-reaches-backoff-through-super.rs")]
fn submodule_import_the_table_does_not_allow_fails(#[case] case: &str) {
    let workspace = Workspace::from_fixture("modules");
    workspace.add_fixture_file(
        &format!("modules-cases/{case}"),
        "crates/goodfetch-engine/src/retry/policy.rs",
    );

    let outcome = workspace.xtask(&["modules"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "forbidden-import",
            "goodfetch-engine/retry::policy->backoff"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::submodule_missing_from_the_table(
    "unlisted-submodule.toml",
    "module-not-listed",
    "goodfetch-engine/retry::policy"
)]
#[case::name_that_is_no_submodule(
    "unknown-submodule.toml",
    "unknown-module",
    "goodfetch-engine/retry::jitter"
)]
#[case::cycle_among_allowed_submodule_imports(
    "submodule-cycle.toml",
    "import-cycle",
    "goodfetch-engine/retry"
)]
fn submodule_table_that_does_not_match_the_module_fails(
    #[case] case: &str,
    #[case] kind: &str,
    #[case] subject: &str,
) {
    let workspace = Workspace::from_fixture("modules");
    workspace.add_fixture_file(&format!("modules-cases/{case}"), MODULES_FILE);

    let outcome = workspace.xtask(&["modules"]);

    assert_eq!(outcome.findings(), vec![finding(kind, subject)]);
    assert_eq!(outcome.code, 1);
}

#[test]
fn crate_without_a_modules_file_fails() {
    let workspace = Workspace::from_fixture("modules");
    workspace.remove(MODULES_FILE);

    let outcome = workspace.xtask(&["modules"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("missing-modules-file", "goodfetch-engine")]
    );
    assert_eq!(outcome.code, 1);
}

/// A module file of `lines` lines: a `//!` header (with `header_extra` on its
/// second line), then one function per line.
fn module_of(lines: usize, header_extra: &str) -> String {
    let mut text = format!("//! Time.\n{header_extra}\n");
    for index in 2..lines {
        writeln!(text, "pub(crate) fn now_{index}() {{}}").unwrap();
    }
    text
}

#[test]
fn file_over_1000_lines_without_a_size_line_fails() {
    let workspace = Workspace::from_fixture("modules");
    workspace.write(CLOCK, module_of(1001, "//!").as_bytes());

    let outcome = workspace.xtask(&["modules"]);

    assert_eq!(outcome.findings(), vec![finding("oversized-file", CLOCK)]);
    assert_eq!(outcome.code, 1);
}

#[test]
fn file_over_1000_lines_with_a_size_line_passes() {
    let workspace = Workspace::from_fixture("modules");
    workspace.write(
        CLOCK,
        module_of(1001, "//! Size: one table of clock sources, kept whole.").as_bytes(),
    );

    let outcome = workspace.xtask(&["modules"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn test_items_do_not_count_toward_1000_lines() {
    let workspace = Workspace::from_fixture("modules");
    let mut text = module_of(990, "//!");
    text.push_str("#[cfg(test)]\nmod tests {\n");
    for index in 0..20 {
        writeln!(text, "    fn case_{index}() {{}}").unwrap();
    }
    text.push_str("}\n");
    workspace.write(CLOCK, text.as_bytes());

    let outcome = workspace.xtask(&["modules"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}
