//! `cargo_consumer` (Seam B, TESTING.md, Installed packages): a temporary
//! consumer project builds against what `cargo package` put in a crate's
//! `.crate` file, so a file the package leaves out fails its build; and
//! `cargo_install` puts both Products' packaged executables in one Cargo
//! root, refusing a crate whose executable another already installed there.

use std::path::{Path, PathBuf};

use goodfetch_test_support::{ConsumerError, TestEnv, cargo_consumer, cargo_install, command};

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

/// The executable file `name` in a Cargo root's `bin` folder.
fn installed(root: &Path, name: &str) -> PathBuf {
    root.join("bin")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// A dependency-free crate named `package` in the test's folder `folder`,
/// whose one executable `executable` prints its name and version, as a
/// Product's executable answers `--version`.
fn fixture_product(env: &TestEnv, folder: &str, package: &str, executable: &str) -> PathBuf {
    let crate_folder = env.folder().join(folder);
    std::fs::create_dir_all(crate_folder.join("src")).unwrap();
    std::fs::write(
        crate_folder.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{package}\"\nversion = \"0.2.0\"\nedition = \"2024\"\n\
             description = \"A fixture Product\"\nlicense = \"MIT OR Apache-2.0\"\n\n\
             [[bin]]\nname = \"{executable}\"\npath = \"src/main.rs\"\n\n[workspace]\n"
        ),
    )
    .unwrap();
    std::fs::write(
        crate_folder.join("src").join("main.rs"),
        format!("fn main() {{\n    println!(\"{executable} 0.2.0\");\n}}\n"),
    )
    .unwrap();
    crate_folder
}

/// Covers: story:11-288
#[test]
fn both_products_install_into_one_cargo_root_and_both_executables_run() {
    let env = TestEnv::new();
    let root = env.folder().join("cargo root");
    let collie = fixture_product(&env, "collie", "goodfetch-collie", "goodfetch-collie");
    let foxhound = fixture_product(&env, "foxhound", "goodfetch-foxhound", "goodfetch-foxhound");

    let installs = [
        cargo_install(&env, &collie, &root).unwrap(),
        cargo_install(&env, &foxhound, &root).unwrap(),
    ];

    for install in &installs {
        assert!(install.success(), "{}", install.stderr());
    }
    for name in ["goodfetch-collie", "goodfetch-foxhound"] {
        command(&env, installed(&root, name))
            .assert()
            .success()
            .stdout(format!("{name} 0.2.0\n"));
    }
}

#[test]
fn a_crate_whose_executable_another_installed_is_refused_in_the_same_root() {
    let env = TestEnv::new();
    let root = env.folder().join("cargo root");
    let collie = fixture_product(&env, "collie", "goodfetch-collie", "goodfetch-collie");
    let impostor = fixture_product(&env, "impostor", "goodfetch-impostor", "goodfetch-collie");
    let first = cargo_install(&env, &collie, &root).unwrap();
    assert!(first.success(), "{}", first.stderr());

    let second = cargo_install(&env, &impostor, &root).unwrap();

    assert!(!second.success());
    assert!(
        second.stderr().contains("already exists"),
        "{}",
        second.stderr()
    );
}
