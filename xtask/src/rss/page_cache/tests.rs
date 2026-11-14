use super::*;

#[test]
fn a_missing_file_fails_naming_it() {
    let home = tempfile::tempdir().expect("a temporary folder");
    let missing = home.path().join("no-such-executable");
    let result = drop_cached(&missing);
    if DROPS {
        let error = result.expect_err("a missing file is no executable to drop");
        assert!(
            format!("{error:#}").contains("no-such-executable"),
            "the error names the file: {error:#}"
        );
    } else {
        result.expect("this OS drops nothing");
    }
}

#[test]
fn an_existing_file_is_dropped() {
    let home = tempfile::tempdir().expect("a temporary folder");
    let file = home.path().join("executable");
    std::fs::write(&file, b"pages").expect("writing the file");
    drop_cached(&file).expect("the file is dropped");
}
