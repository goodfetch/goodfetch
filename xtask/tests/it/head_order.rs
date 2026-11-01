//! The incremental gate reruns Vitest, Playwright and pytest in the head's
//! order (ADR 0135, the owner's F8, option B): a part runs `just test
//! <suite> --shuffled-all <crates>`, the repository's own recipe, whose
//! shuffling runners take no member filter and the seed `ci/order-seed.sh`
//! gives the tree under test. These tests run that recipe in a fixture,
//! with stand-ins for cargo and pnpm and the repository's installed pytest.
//! The local gate is repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use crate::workspace::Workspace;

/// Two tests that share state: `test_second` passes only after
/// `test_first`, so it fails in about half the orders a seed gives.
const ORDER_DEPENDENT: &str = r#""""Two tests that share state."""

SEEN: list[int] = []


def test_first() -> None:
    """Records that it ran."""
    SEEN.append(1)


def test_second() -> None:
    """Passes only after test_first."""
    assert SEEN == [1], "test_second ran before test_first"
"#;

pub(crate) struct Fixture {
    pub(crate) workspace: Workspace,
    stand_ins: PathBuf,
}

/// The repository's justfile and `ci/` in a workspace with two uv members:
/// `ordered`, whose tests depend on their order, and `other`. cargo and
/// pnpm are stand-ins that record their arguments; cargo exits with the
/// status [`Fixture::fail`] gave its subcommand. uv's stand-in records
/// them too and runs `uv run pytest` with the repository's pytest.
pub(crate) fn fixture() -> Fixture {
    let workspace = Workspace::from_fixture("empty");
    workspace.link_installed_runners();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    std::fs::copy(
        repository.join("justfile"),
        workspace.path().join("justfile"),
    )
    .unwrap();
    std::os::unix::fs::symlink(repository.join("ci"), workspace.path().join("ci")).unwrap();
    workspace.write(".gitignore", b"node_modules\n.venv\ntarget\nstand-ins\n");
    workspace.write(
        "pyproject.toml",
        b"[tool.pytest.ini_options]\ntestpaths = [\"ordered\", \"other\"]\n",
    );
    for member in ["ordered", "other"] {
        workspace.write(
            &format!("{member}/pyproject.toml"),
            format!("[project]\nname = \"{member}\"\nversion = \"0.0.0\"\n").as_bytes(),
        );
    }
    workspace.write("ordered/tests/test_shared.py", ORDER_DEPENDENT.as_bytes());
    workspace.write(
        "other/tests/test_other.py",
        b"\"\"\"A test of another member.\"\"\"\n\n\ndef test_other() -> None:\n    \"\"\"Passes.\"\"\"\n",
    );
    let stand_ins = workspace.path().join("stand-ins");
    std::fs::create_dir_all(&stand_ins).unwrap();
    let calls = stand_ins.join("calls");
    let scripts = [
        (
            "cargo",
            format!(
                "#!/bin/sh\necho \"cargo $*\" >>\"{calls}\"\nmkdir -p target\necho '{{}}' >target/test-tools.json\nif [ -f \"{stand_ins}/$1.status\" ]; then exit \"$(cat \"{stand_ins}/$1.status\")\"; fi\n",
                calls = calls.display(),
                stand_ins = stand_ins.display()
            ),
        ),
        (
            "pnpm",
            format!("#!/bin/sh\necho \"pnpm $*\" >>\"{}\"\n", calls.display()),
        ),
        (
            "uv",
            format!(
                "#!/bin/sh\necho \"uv $*\" >>\"{calls}\"\nif [ \"$1\" = run ] && [ \"$2\" = pytest ]; then\n  shift 2\n  exec \"$PWD/.venv/bin/pytest\" -p no:cacheprovider \"$@\"\nfi\n",
                calls = calls.display()
            ),
        ),
    ];
    for (name, script) in scripts {
        let path = stand_ins.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    Fixture {
        workspace,
        stand_ins,
    }
}

pub(crate) struct Run {
    pub(crate) code: Option<i32>,
    pub(crate) output: String,
    pub(crate) calls: Vec<String>,
}

impl Fixture {
    /// Makes the cargo stand-in exit with `status` when its subcommand is
    /// `subcommand` (`insta` for `cargo insta test`, `test` for `cargo test`).
    pub(crate) fn fail(&self, subcommand: &str, status: &str) {
        std::fs::write(self.stand_ins.join(format!("{subcommand}.status")), status).unwrap();
    }

    /// Runs the real `just test <args>` with the stand-ins first on PATH.
    pub(crate) fn test(&self, args: &[&str]) -> Run {
        let _ = std::fs::remove_file(self.stand_ins.join("calls"));
        let path = format!(
            "{}:{}",
            self.stand_ins.display(),
            std::env::var("PATH").unwrap()
        );
        let output = self
            .workspace
            .command("just")
            .arg("test")
            .args(args)
            .env("PATH", path)
            .env("HOME", self.workspace.path())
            .output()
            .unwrap();
        Run {
            code: output.status.code(),
            output: format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            calls: std::fs::read_to_string(self.stand_ins.join("calls"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect(),
        }
    }

    /// The order seed of the tree under test, as `ci/order-seed.sh` gives it.
    fn seed(&self) -> String {
        let output = self
            .workspace
            .command(self.workspace.path().join("ci/order-seed.sh"))
            .env("PATH", std::env::var("PATH").unwrap())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
}

/// `--shuffled-all` runs Vitest, Playwright and pytest with no member
/// filter and the head's seed, as over the whole workspace, and narrows
/// nextest alone to the crates given.
#[test]
fn shuffled_all_runs_the_shuffling_runners_unfiltered_in_the_heads_order() {
    let fixture = fixture();
    fixture.workspace.commit_all();
    let seed = fixture.seed();

    let run = fixture.test(&["pre-merge", "--shuffled-all", "a-crate", "other"]);

    let ran = |prefix: &str| -> Vec<&String> {
        run.calls
            .iter()
            .filter(|call| call.starts_with(prefix))
            .collect()
    };
    assert!(
        ran("cargo insta test -p a-crate ").len() == 1,
        "{:?}\n{}",
        run.calls,
        run.output
    );
    assert_eq!(
        ran("pnpm exec vitest run"),
        [&format!(
            "pnpm exec vitest run --sequence.shuffle.tests --sequence.shuffle.files --sequence.seed={seed}"
        )]
    );
    assert_eq!(
        ran("pnpm exec playwright test"),
        [&format!(
            "pnpm exec playwright test --pass-with-no-tests --shuffle={seed}"
        )]
    );
    assert_eq!(
        ran("uv run pytest"),
        [&format!("uv run pytest -p randomly --randomly-seed={seed}")]
    );
}

/// A test that passes in one tree's order and fails in another's fails the
/// head-order rerun on the second tree, while the part without it (the
/// gate before option B, which reused `ordered`) passes there.
#[test]
fn an_order_dependent_test_fails_the_head_order_rerun_of_a_reused_member() {
    let fixture = fixture();
    let mut passed_in_its_order = false;
    let mut failed_in_its_order = false;
    for n in 0..40 {
        fixture
            .workspace
            .write("tree.txt", format!("{n}\n").as_bytes());
        fixture.workspace.commit_all();

        let rerun = fixture.test(&["pre-merge", "--shuffled-all", "other"]);

        if rerun.code == Some(0) {
            passed_in_its_order = true;
        } else {
            assert!(
                rerun.output.contains("test_second ran before test_first"),
                "{}",
                rerun.output
            );
            let without = fixture.test(&["pre-merge", "other"]);
            assert_eq!(without.code, Some(0), "{}", without.output);
            failed_in_its_order = true;
        }
        if passed_in_its_order && failed_in_its_order {
            return;
        }
    }
    panic!(
        "40 trees gave no pair of orders: passed {passed_in_its_order}, failed {failed_in_its_order}"
    );
}
