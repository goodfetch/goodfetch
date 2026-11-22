//! `ci/mutants-typescript.sh`, StrykerJS over the TypeScript SDK's
//! hand-written runtime (ADR 0072; TESTING.md, Feature coverage): given a
//! base, `just mutants-typescript` mutates only the runtime lines the change
//! touched since it, never a `generated/` file (ADR 0099) nor a test; without
//! one, `just mutants-sdks typescript` mutates the whole runtime. Either way
//! the test tools are built once its checks pass, the tests the quarantine
//! list names are skipped, StrykerJS's leftover
//! setup files are removed, and the surviving mutants are listed in the
//! report's `summary.md` for the pull request's Evidence section, failing
//! nothing. The script runs on a fixture repository through stand-ins for
//! `pnpm`, which records the StrykerJS run and writes its JSON report, and
//! `cargo`, which answers the quarantine's exclusions, and `just`, which
//! records the test tools' build. Repository tooling,
//! so the module declares no Feature.
#![cfg(unix)]

use std::path::Path;

use crate::workspace::{Workspace, write_executable};

/// The quarantine's exclusions the `cargo` stand-in answers.
const EXCLUSIONS: &str = r#"{"vitest":["--testNamePattern=^(?!.*a quarantined test)"]}"#;

/// The StrykerJS JSON report the `pnpm` stand-in writes: one mutant
/// survived, one was killed and no test reaches the third.
const REPORT: &str = r#"{"schemaVersion":"2","thresholds":{"high":80,"low":60},"files":{
"sdks/typescript/collie/src/run.ts":{"language":"typescript","source":"","mutants":[
{"id":"1","mutatorName":"ConditionalExpression","replacement":"true","status":"Survived","location":{"start":{"line":2,"column":7},"end":{"line":2,"column":12}}},
{"id":"2","mutatorName":"StringLiteral","replacement":"\"\"","status":"Killed","location":{"start":{"line":4,"column":1},"end":{"line":4,"column":5}}},
{"id":"3","mutatorName":"BlockStatement","replacement":"{}","status":"NoCoverage","location":{"start":{"line":5,"column":1},"end":{"line":5,"column":3}}}]}}}"#;

/// What a run of the script did.
struct Run {
    code: Option<i32>,
    /// The `pnpm` stand-in's record: its arguments, the name pattern and the
    /// report folder and test tools' manifest it was given; `None` when
    /// StrykerJS never ran.
    stryker: Option<String>,
    /// The `just` stand-in's record: its arguments and the suite it was
    /// given; `None` when no test tool was built.
    just: Option<String>,
    stdout: String,
    stderr: String,
}

/// A repository with the SDK's runtime, a generated file, tests and a test
/// helper committed as the base, and stand-ins in `stand-in/`; returns it
/// and the base commit.
fn sdk() -> (Workspace, String) {
    let workspace = Workspace::from_fixture("empty");
    let collie = "sdks/typescript/collie";
    workspace.write(
        &format!("{collie}/src/run.ts"),
        b"one\ntwo\nthree\nfour\nfive\nsix\nseven\n",
    );
    workspace.write(&format!("{collie}/src/sub/deep.ts"), b"one\ntwo\n");
    workspace.write(&format!("{collie}/src/two w\u{f6}rds.ts"), b"one\n");
    workspace.write(&format!("{collie}/src/gone.ts"), b"one\n");
    workspace.write(&format!("{collie}/src/generated/handles.ts"), b"one\n");
    workspace.write(&format!("{collie}/src/run.test.ts"), b"one\n");
    workspace.write(&format!("{collie}/tests/helper.ts"), b"one\n");
    workspace.write(".gitignore", b"/stand-in/\n/target/\n");
    let stand_in = workspace.path().join("stand-in");
    std::fs::create_dir_all(&stand_in).unwrap();
    write_executable(
        &stand_in.join("pnpm"),
        &format!(
            "#!/bin/sh\n\
             printf 'args=%s\\npattern=%s\\nreports=%s\\ntools=%s\\n' \"$*\" \"${{GOODFETCH_STRYKER_NAME_PATTERN-}}\" \"$GOODFETCH_STRYKER_REPORTS\" \"$GOODFETCH_TEST_TOOLS\" > \"{record}\"\n\
             mkdir -p \"$GOODFETCH_STRYKER_REPORTS\"\n\
             cat > \"$GOODFETCH_STRYKER_REPORTS/report.json\" <<'EOF'\n{REPORT}\nEOF\n\
             touch stryker-setup-1.js\n",
            record = stand_in.join("stryker").display(),
        ),
    );
    write_executable(
        &stand_in.join("cargo"),
        &format!("#!/bin/sh\necho '{EXCLUSIONS}'\n"),
    );
    write_executable(
        &stand_in.join("just"),
        &format!(
            "#!/bin/sh\nprintf '%s suite=%s\\n' \"$*\" \"$GOODFETCH_TEST_SUITE\" > \"{record}\"\n",
            record = stand_in.join("just.record").display(),
        ),
    );
    let base = workspace.commit_all();
    (workspace, base)
}

/// Changes line 2 of `run.ts`, drops its line 4 and changes its line 6,
/// now 5; changes line 1 of `sub/deep.ts` and of the file whose name git
/// quotes; adds a three-line `new.ts`, deletes `gone.ts`, and changes the
/// generated file, the test and the helper; commits it all.
fn change(workspace: &Workspace) {
    let collie = "sdks/typescript/collie";
    workspace.write(
        &format!("{collie}/src/run.ts"),
        b"one\nTWO\nthree\nfive\nSIX\nseven\n",
    );
    workspace.write(&format!("{collie}/src/sub/deep.ts"), b"ONE\ntwo\n");
    workspace.write(&format!("{collie}/src/two w\u{f6}rds.ts"), b"ONE\n");
    workspace.write(&format!("{collie}/src/new.ts"), b"one\ntwo\nthree\n");
    workspace.remove(&format!("{collie}/src/gone.ts"));
    workspace.write(&format!("{collie}/src/generated/handles.ts"), b"ONE\n");
    workspace.write(&format!("{collie}/src/run.test.ts"), b"ONE\n");
    workspace.write(&format!("{collie}/tests/helper.ts"), b"ONE\n");
    workspace.commit_all();
}

/// Runs the repository's `ci/mutants-typescript.sh` in the workspace with
/// `args`, the stand-ins first on `PATH`.
fn mutants_typescript(workspace: &Workspace, args: &[&str]) -> Run {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/mutants-typescript.sh");
    let path = std::env::join_paths(
        std::iter::once(workspace.path().join("stand-in"))
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = workspace
        .command("bash")
        .arg(script)
        .args(args)
        .env("PATH", path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    Run {
        code: output.status.code(),
        stryker: std::fs::read_to_string(workspace.path().join("stand-in/stryker")).ok(),
        just: std::fs::read_to_string(workspace.path().join("stand-in/just.record")).ok(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The test tools' manifest the script hands StrykerJS, built through `just`
/// for the Pre-merge suite as under `just test`.
fn tools(workspace: &Workspace) -> String {
    let root = workspace.path().canonicalize().unwrap();
    format!("tools={}/target/test-tools.json\n", root.display())
}

const TOOLS_BUILT: Option<&str> = Some("_test-tools suite=pre-merge\n");

/// The summary the script wrote in `folder`.
fn summary(workspace: &Workspace, folder: &str) -> String {
    workspace.read_text(&format!("{folder}/summary.md"))
}

/// The survivors' list both runs write, from [`REPORT`].
const SURVIVORS: &str = "### Surviving TypeScript mutants\n\
    \n\
    - `sdks/typescript/collie/src/run.ts:2:7` ConditionalExpression: `true`, survived\n\
    - `sdks/typescript/collie/src/run.ts:5:1` BlockStatement: `{}`, no test reaches it\n";

#[test]
fn a_change_mutates_only_the_runtime_lines_it_touched_and_lists_the_survivors_failing_nothing() {
    let (workspace, base) = sdk();
    change(&workspace);

    let run = mutants_typescript(&workspace, &[&base]);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.stryker,
        Some(
            "args=exec stryker run sdks/typescript/stryker.config.mjs --mutate \
             sdks/typescript/collie/src/new.ts:1-3,\
             sdks/typescript/collie/src/run.ts:2-2,\
             sdks/typescript/collie/src/run.ts:5-5,\
             sdks/typescript/collie/src/sub/deep.ts:1-1,\
             sdks/typescript/collie/src/two w\u{f6}rds.ts:1-1\n\
             pattern=^(?!.*a quarantined test)\n\
             reports=target/mutants-typescript\n"
                .to_owned()
                + &tools(&workspace)
        )
    );
    assert_eq!(run.just.as_deref(), TOOLS_BUILT);
    assert_eq!(summary(&workspace, "target/mutants-typescript"), SURVIVORS);
    assert!(run.stdout.contains(SURVIVORS), "{}", run.stdout);
    assert!(!workspace.path().join("stryker-setup-1.js").exists());
}

#[test]
fn a_change_that_touches_no_runtime_line_runs_no_mutant() {
    let (workspace, base) = sdk();
    let collie = "sdks/typescript/collie";
    workspace.write(&format!("{collie}/src/generated/handles.ts"), b"ONE\n");
    workspace.write(&format!("{collie}/src/run.test.ts"), b"ONE\n");
    workspace.write(&format!("{collie}/tests/helper.ts"), b"ONE\n");
    workspace.remove(&format!("{collie}/src/gone.ts"));
    workspace.commit_all();

    let run = mutants_typescript(&workspace, &[&base]);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(run.stryker, None);
    assert_eq!(run.just, None);
    assert!(
        run.stdout
            .contains("touches no line of the TypeScript SDK's runtime"),
        "{}",
        run.stdout
    );
}

#[test]
fn uncommitted_runtime_changes_are_refused_before_any_build_or_mutant() {
    let (workspace, base) = sdk();
    workspace.write("sdks/typescript/collie/src/run.ts", b"uncommitted\n");

    let run = mutants_typescript(&workspace, &[&base]);

    assert_eq!(run.code, Some(2), "{}{}", run.stdout, run.stderr);
    assert_eq!(run.stryker, None);
    assert_eq!(run.just, None);
    assert!(
        run.stderr.contains("commit the TypeScript changes first"),
        "{}",
        run.stderr
    );
}

#[test]
fn without_a_base_the_whole_runtime_is_mutated_into_the_full_runs_folder() {
    let (workspace, _) = sdk();

    let run = mutants_typescript(&workspace, &[]);

    assert_eq!(run.code, Some(0), "{}{}", run.stdout, run.stderr);
    assert_eq!(
        run.stryker,
        Some(
            "args=exec stryker run sdks/typescript/stryker.config.mjs\n\
             pattern=^(?!.*a quarantined test)\n\
             reports=target/mutants-sdks/typescript\n"
                .to_owned()
                + &tools(&workspace)
        )
    );
    assert_eq!(run.just.as_deref(), TOOLS_BUILT);
    assert_eq!(
        summary(&workspace, "target/mutants-sdks/typescript"),
        SURVIVORS
    );
}
