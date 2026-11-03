use super::{RepoPath, literals};

#[test]
fn a_literal_names_a_file_by_its_name_or_a_folder_that_holds_it() {
    let literals = literals(&RepoPath::new("chromium/pins/linux.json"));

    for wanted in [
        "linux.json\"",
        "\"chromium/pins\"",
        "\"chromium/",
        "'chromium/pins/",
    ] {
        assert!(literals.iter().any(|literal| literal == wanted), "{wanted}");
    }
}
