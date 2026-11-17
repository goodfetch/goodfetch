//! `cargo_consumer` (Seam B, TESTING.md, Installed packages): a temporary
//! consumer project builds against what `cargo package` put in a crate's
//! `.crate` file, so a file the package leaves out fails its build.

use std::path::{Path, PathBuf};

use goodfetch_test_support::{ConsumerError, TestEnv, cargo_consumer, command};

/// The fixture crate, a library whose function lives in `src/greeting.rs`.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/it/fixtures/cargo-consumer"
);

/// The consumer's `main.rs`: it prints the fixture's greeting.
const MAIN: &str = r#"fn main() {
    println!("{}", goodfetch_consumer_fixture::greet("consumer"));
}
"#;

/// A copy of the fixture crate in the test's folder, since `cargo package`
/// writes its lockfile beside the manifest; `exclude`, when given, is the
/// manifest's `package.exclude`.
fn fixture_copy(env: &TestEnv, exclude: Option<&str>) -> PathBuf {
    let copy = env.folder().join("fixture crate");
    std::fs::create_dir_all(copy.join("src")).unwrap();
    for file in ["src/lib.rs", "src/greeting.rs"] {
        std::fs::copy(Path::new(FIXTURE).join(file), copy.join(file)).unwrap();
    }
    let manifest = std::fs::read_to_string(Path::new(FIXTURE).join("Cargo.toml")).unwrap();
    let manifest = match exclude {
        Some(file) => manifest.replace(
            "[package]\n",
            &format!("[package]\nexclude = [\"{file}\"]\n"),
        ),
        None => manifest,
    };
    std::fs::write(copy.join("Cargo.toml"), manifest).unwrap();
    copy
}

#[test]
fn the_consumer_builds_against_the_packaged_crate_and_runs() {
    let env = TestEnv::new();
    let package = fixture_copy(&env, None);

    let consumer = cargo_consumer(&env, &package, MAIN).unwrap();

    assert!(consumer.build().success(), "{}", consumer.build().stderr());
    command(&env, consumer.executable())
        .assert()
        .success()
        .stdout("hello, consumer\n");
}

#[test]
fn a_file_the_package_leaves_out_fails_the_consumer_s_build() {
    let env = TestEnv::new();
    let package = fixture_copy(&env, Some("src/greeting.rs"));

    let consumer = cargo_consumer(&env, &package, MAIN).unwrap();

    assert!(!consumer.build().success());
    assert!(
        consumer.build().stderr().contains("greeting"),
        "{}",
        consumer.build().stderr()
    );
}

#[test]
fn a_folder_with_no_crate_fails_to_package() {
    let env = TestEnv::new();

    let error = cargo_consumer(&env, &env.folder().join("no crate"), MAIN).unwrap_err();

    assert!(matches!(error, ConsumerError::Package { .. }), "{error}");
}
