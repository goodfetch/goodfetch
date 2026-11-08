//! Each Product's executable crate (ADRs 0084, 0087, 0090): its package's
//! name, which is its binary's too, and its folder from the workspace's.
//! Every command that names an executable crate, or a file inside one,
//! reads it here, so moving a crate is one edit.

/// Collie's executable crate's folder, as a literal `concat!` extends into
/// the path of a file inside it.
macro_rules! collie_folder {
    () => {
        "crates/goodfetch-collie"
    };
}
pub(crate) use collie_folder;

/// Foxhound's executable crate's folder, as a literal `concat!` extends
/// into the path of a file inside it; it lies under `crates/foxhound/`,
/// with every crate only Foxhound links (ADR 0087).
macro_rules! foxhound_folder {
    () => {
        "crates/foxhound/goodfetch-foxhound"
    };
}
pub(crate) use foxhound_folder;

/// One Product's executable crate.
#[derive(Clone, Copy)]
pub(crate) struct Executable {
    /// Its package's name, and its binary's.
    pub(crate) package: &'static str,
    /// Its folder, from the workspace's.
    pub(crate) folder: &'static str,
}

/// Collie's executable crate, whose `executable` module both Products'
/// executables start from (ADR 0087).
pub(crate) const COLLIE: Executable = Executable {
    package: "goodfetch-collie",
    folder: collie_folder!(),
};

/// Foxhound's executable crate.
pub(crate) const FOXHOUND: Executable = Executable {
    package: "goodfetch-foxhound",
    folder: foxhound_folder!(),
};

/// Each Product's executable crate.
pub(crate) const EXECUTABLES: [Executable; 2] = [COLLIE, FOXHOUND];

/// Each Product's executable crate's package name.
pub(crate) const PACKAGES: [&str; 2] = [COLLIE.package, FOXHOUND.package];

/// Whether `package` is a Product's executable crate.
pub(crate) fn is_executable(package: &str) -> bool {
    PACKAGES.contains(&package)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "a test of the workspace's own manifests; the blocking-fs ban targets async crates (STYLE.md)"
    )]
    fn each_executable_crate_is_the_package_its_folder_holds() {
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");

        let packages: Vec<String> = EXECUTABLES
            .iter()
            .map(|executable| {
                let manifest =
                    std::fs::read_to_string(workspace.join(executable.folder).join("Cargo.toml"))
                        .unwrap();
                let manifest: toml::Table = toml::from_str(&manifest).unwrap();
                manifest["package"]["name"].as_str().unwrap().to_owned()
            })
            .collect();

        assert_eq!(packages, PACKAGES);
    }
}
