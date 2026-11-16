use std::fs;
use std::path::Path;

use crate::env::TestEnv;
use crate::facts::CHROME_FOR_TESTING_BUILD;
use crate::variables::Variables;

use super::build_copy;

const PLATFORM: &str = "chrome-linux64";

/// A stand-in build at `folder`: an executable, a file in a folder of its
/// own and a symbolic link to it.
fn lay_out_build(folder: &Path) {
    fs::create_dir_all(folder.join("locales")).unwrap();
    fs::write(folder.join("chrome"), "the executable").unwrap();
    fs::write(folder.join("locales").join("fr.pak"), "the locale").unwrap();
    super::symlink(Path::new("chrome"), &folder.join("chrome-wrapper")).unwrap();
}

/// The names in `folder`, sorted.
fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_builds_copy_is_laid_out_whole_once_and_kept_for_later_tests() {
    let env = TestEnv::create("browsers::tests::copy", &Variables::default()).unwrap();
    let source = env.folder().join("source");
    lay_out_build(&source);
    let tests_folder = env.folder().join("tests");
    fs::create_dir(&tests_folder).unwrap();

    let first = build_copy(&source, &tests_folder, PLATFORM).unwrap();
    fs::write(first.join("laid-out-once"), "").unwrap();
    let second = build_copy(&source, &tests_folder, PLATFORM).unwrap();

    let name = format!("goodfetch-chrome-for-testing-{CHROME_FOR_TESTING_BUILD}-{PLATFORM}");
    assert_eq!(first, tests_folder.join(&name));
    assert_eq!(second, first);
    assert!(second.join("laid-out-once").is_file());
    assert_eq!(
        fs::read_to_string(second.join("chrome")).unwrap(),
        "the executable"
    );
    assert_eq!(
        fs::read_to_string(second.join("locales").join("fr.pak")).unwrap(),
        "the locale"
    );
    assert_eq!(
        fs::read_link(second.join("chrome-wrapper")).unwrap(),
        Path::new("chrome")
    );
    assert_eq!(names(&tests_folder), [name.clone(), format!("{name}.lock")]);
}

/// A test killed while it laid out the copy leaves a part of one, which no
/// teardown removes; the next layout removes it.
#[test]
fn a_part_left_by_a_killed_layout_goes_with_the_next_layout() {
    let env = TestEnv::create("browsers::tests::part", &Variables::default()).unwrap();
    let source = env.folder().join("source");
    lay_out_build(&source);
    let tests_folder = env.folder().join("tests");
    let name = format!("goodfetch-chrome-for-testing-{CHROME_FOR_TESTING_BUILD}-{PLATFORM}");
    let part = tests_folder.join(format!(".{name}-killed")).join(PLATFORM);
    fs::create_dir_all(&part).unwrap();
    fs::write(part.join("chrome"), "the executable, half").unwrap();

    build_copy(&source, &tests_folder, PLATFORM).unwrap();

    assert_eq!(names(&tests_folder), [name.clone(), format!("{name}.lock")]);
}
