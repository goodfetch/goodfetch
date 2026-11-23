//! The stand-ins table the build script writes under `OUT_DIR`, read as
//! the library includes it.

/// The table names no checkout: each stand-in file's path is built when
/// the library is compiled, from the crate's folder as Cargo gives it to
/// the compiler, so a checkout that builds in a build folder another
/// checkout built last includes its own files, and still builds once that
/// other checkout is removed (issue 2487).
#[test]
fn the_stand_ins_table_names_no_checkout() {
    let table = include_str!(concat!(env!("OUT_DIR"), "/stand_ins.rs"));
    let checkout = env!("CARGO_MANIFEST_DIR");

    let with_checkout: Vec<&str> = table.lines().filter(|row| row.contains(checkout)).collect();
    let files: Vec<&str> = table
        .lines()
        .filter(|row| row.contains("include_bytes!"))
        .collect();

    assert!(with_checkout.is_empty(), "{with_checkout:#?}");
    assert!(!files.is_empty(), "the table includes no stand-in file");
}
