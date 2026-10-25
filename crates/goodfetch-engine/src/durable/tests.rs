#![expect(
    clippy::disallowed_methods,
    reason = "a test sets its folders up on its own thread, outside any async worker"
)]

use super::create_folder;

#[test]
fn a_folder_is_created_with_each_missing_folder_above_it_and_an_existing_one_is_kept() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("0.0").join("runs").join("cut");
    std::fs::create_dir(root.path().join("0.0")).unwrap();
    std::fs::write(root.path().join("0.0").join("kept"), "kept").unwrap();

    create_folder(&folder).unwrap();
    create_folder(&folder).unwrap();

    assert_eq!(
        (
            folder.is_dir(),
            std::fs::read_to_string(root.path().join("0.0").join("kept")).unwrap(),
        ),
        (true, "kept".to_owned())
    );
}
