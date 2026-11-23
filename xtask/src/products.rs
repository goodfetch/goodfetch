//! Each Product's executable crate (ADRs 0084, 0087, 0090): its package's
//! name, which is its binary's too, and its folder from the workspace's.
//! Every command that names an executable crate, or a file inside one,
//! reads it here, so moving a crate is one edit. Each Product's names on
//! every surface are here too.

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

/// One of the two Products (ADR 0084), as a docs page or a package names
/// it: `collie` or `foxhound`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Product {
    Collie,
    Foxhound,
}

impl Product {
    /// Both Products, Collie first: a test runs on Collie unless it needs
    /// Foxhound (ADR 0094).
    pub(crate) const ALL: [Self; 2] = [Self::Collie, Self::Foxhound];

    /// Its name as a page, a folder or a manifest writes it.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Collie => "collie",
            Self::Foxhound => "foxhound",
        }
    }

    /// The Product a word names, as a page's front matter or a package's
    /// folder writes it.
    pub(crate) fn named(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|product| product.key() == word)
    }

    /// Its executable crate.
    pub(crate) fn executable(self) -> Executable {
        match self {
            Self::Collie => COLLIE,
            Self::Foxhound => FOXHOUND,
        }
    }

    /// The Product whose executable crate is the package `package`.
    pub(crate) fn of_package(package: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|product| product.executable().package == package)
    }

    /// Whether its executable has everything `other`'s has: Foxhound's
    /// contains Collie's (ADR 0084), and each has its own.
    pub(crate) fn contains(self, other: Self) -> bool {
        self == other || other == Self::Collie
    }

    pub(crate) fn other(self) -> Self {
        match self {
            Self::Collie => Self::Foxhound,
            Self::Foxhound => Self::Collie,
        }
    }

    /// Its names on every surface (ADR 0090).
    pub(crate) fn names(self) -> &'static Names {
        match self {
            Self::Collie => &COLLIE_NAMES,
            Self::Foxhound => &FOXHOUND_NAMES,
        }
    }
}

/// One Product's names (ADR 0090), which the docs write through markers
/// and no code of the other Product's docs may carry (ADR 0097).
pub(crate) struct Names {
    /// The executable's, and the PyPI, crates.io, Homebrew, skill and MCP
    /// server name.
    pub(crate) executable: &'static str,
    /// The npm package's.
    pub(crate) npm: &'static str,
    /// The Python and Rust import's.
    pub(crate) import: &'static str,
    /// The Connection type's.
    pub(crate) connection: &'static str,
    /// The variable naming the engine's executable.
    pub(crate) engine_path_variable: &'static str,
    /// The installer's address, without its `.sh` or `.ps1`.
    pub(crate) installer: &'static str,
    /// The container image's.
    pub(crate) image: &'static str,
}

impl Names {
    /// Each name with the marker the docs write it through, `{{<marker>}}`.
    pub(crate) fn by_marker(&self) -> [(&'static str, &'static str); 7] {
        [
            ("executable", self.executable),
            ("npm", self.npm),
            ("import", self.import),
            ("connection", self.connection),
            ("engine_path_variable", self.engine_path_variable),
            ("installer", self.installer),
            ("image", self.image),
        ]
    }
}

const COLLIE_NAMES: Names = Names {
    executable: COLLIE.package,
    npm: "@goodfetch/collie",
    import: "goodfetch_collie",
    connection: "Collie",
    engine_path_variable: "GOODFETCH_COLLIE_ENGINE_PATH",
    installer: "https://goodfetch.dev/install/collie",
    image: "ghcr.io/goodfetch/collie",
};

const FOXHOUND_NAMES: Names = Names {
    executable: FOXHOUND.package,
    npm: "@goodfetch/foxhound",
    import: "goodfetch_foxhound",
    connection: "Foxhound",
    engine_path_variable: "GOODFETCH_FOXHOUND_ENGINE_PATH",
    installer: "https://goodfetch.dev/install/foxhound",
    image: "ghcr.io/goodfetch/foxhound",
};

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
