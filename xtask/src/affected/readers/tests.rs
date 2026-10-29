use super::{RepoPath, literals};

#[test]
fn a_literal_names_a_file_by_its_name_or_a_folder_that_holds_it() {
    let literals = literals(&RepoPath::new("docs/adr/0135.md"));

    for wanted in ["0135.md\"", "\"docs/adr\"", "\"docs/", "'docs/adr/"] {
        assert!(literals.iter().any(|literal| literal == wanted), "{wanted}");
    }
}
