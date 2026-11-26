//! The fixed deny list (ADR 0033, spec #596): the paths a Plugin can never be
//! granted, whatever its manifest declares and Operator policy approves.
//! Each entry is data with a rule name, which one probe of the probe table
//! names (ADR 0067).
//!
//! An entry lies under a base folder resolved per host and user, or is a
//! name that may sit in any folder (`.env` files, `.git/hooks`).

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// An OS a Plugin runs on, each with its own appliers and probe tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Linux,
    MacOs,
    Windows,
}

impl Host {
    /// The OS this crate is built for, none on an OS no Plugin runs on.
    pub fn current() -> Option<Self> {
        if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else if cfg!(target_os = "macos") {
            Some(Self::MacOs)
        } else if cfg!(windows) {
            Some(Self::Windows)
        } else {
            None
        }
    }
}

/// The OSes an entry applies on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
    /// Linux and macOS, whose home folders share these names.
    Unix,
    Every,
}

impl Os {
    /// Whether the entry applies on `host`.
    pub fn includes(self, host: Host) -> bool {
        match self {
            Self::Every => true,
            Self::Linux => host == Host::Linux,
            Self::MacOs => host == Host::MacOs,
            Self::Windows => host == Host::Windows,
            Self::Unix => host != Host::Windows,
        }
    }
}

/// The folder an entry's path is relative to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    /// The user's home folder.
    Home,
    /// The Goodfetch home both Products share (ADR 0098): the engine's own
    /// files, the Plugin store and every other Plugin's folders.
    GoodfetchHome,
    /// `%APPDATA%` on Windows.
    AppData,
    /// `%LOCALAPPDATA%` on Windows.
    LocalAppData,
    /// Any folder: the entry is a name, or a short path of names, wherever
    /// it lies.
    Anywhere,
}

/// One path a Plugin can never be granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Denied {
    /// The rule's name, which the probe table names.
    pub rule: &'static str,
    pub base: Base,
    /// Relative to `base`, `/`-separated; empty for the base itself.
    pub path: &'static str,
    pub os: Os,
}

/// The base folders of one host and user.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Places {
    pub home: PathBuf,
    pub goodfetch_home: PathBuf,
    pub app_data: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
}

impl Denied {
    /// The entry's path on this host, none for an entry of the `Anywhere`
    /// base or of a Windows folder the host does not have.
    pub fn resolve(&self, places: &Places) -> Option<PathBuf> {
        let base = match self.base {
            Base::Home => &places.home,
            Base::GoodfetchHome => &places.goodfetch_home,
            Base::AppData => places.app_data.as_ref()?,
            Base::LocalAppData => places.local_app_data.as_ref()?,
            Base::Anywhere => return None,
        };
        Some(
            self.path
                .split('/')
                .filter(|name| !name.is_empty())
                .fold(base.clone(), |path, name| path.join(name)),
        )
    }

    /// Whether granting `path` would grant this entry: `path` is the entry,
    /// lies inside it or holds it. An `Anywhere` entry is matched on the
    /// names of `path` itself; what a granted folder holds is
    /// [`crate::policy`]'s walk.
    pub fn covers(&self, path: &Path, places: &Places) -> bool {
        match self.resolve(places) {
            Some(denied) => path.starts_with(&denied) || denied.starts_with(path),
            None => self.base == Base::Anywhere && self.names_in(path),
        }
    }

    /// Whether `path`'s names hold this `Anywhere` entry's names in a row,
    /// the last of them as `.env` files are named (`.env`, `.env.local`).
    pub(crate) fn names_in(&self, path: &Path) -> bool {
        let wanted: Vec<&str> = self.path.split('/').collect();
        let names: Vec<&str> = path
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => name.to_str(),
                _ => None,
            })
            .collect();
        names.windows(wanted.len()).any(|window| {
            window
                .iter()
                .zip(&wanted)
                .enumerate()
                .all(|(index, (name, want))| {
                    let last = index + 1 == wanted.len();
                    *name == *want || (last && *want == ".env" && name.starts_with(".env."))
                })
        })
    }
}

/// The entries that apply on `host`.
pub fn on(host: Host) -> impl Iterator<Item = &'static Denied> {
    DENY_LIST
        .iter()
        .filter(move |denied| denied.os.includes(host))
}

const fn entry(rule: &'static str, base: Base, path: &'static str, os: Os) -> Denied {
    Denied {
        rule,
        base,
        path,
        os,
    }
}

/// The fixed deny list, in the spec's order: the engine's own files,
/// credential files, keychains, browser profiles of every browser ADR 0056
/// reads, shell startup files and version-control hooks.
pub const DENY_LIST: &[Denied] = &[
    // The engine's own files and every other Plugin's folders: the whole
    // Goodfetch home, of which only the Plugin's own bundle, runtime and
    // instance folders are granted.
    entry("deny.goodfetch_home", Base::GoodfetchHome, "", Os::Every),
    // Credential files.
    entry("deny.ssh", Base::Home, ".ssh", Os::Every),
    entry("deny.aws", Base::Home, ".aws", Os::Every),
    entry("deny.azure", Base::Home, ".azure", Os::Every),
    entry("deny.gcloud", Base::Home, ".config/gcloud", Os::Unix),
    entry("deny.gcloud_windows", Base::AppData, "gcloud", Os::Windows),
    entry("deny.kube", Base::Home, ".kube", Os::Every),
    entry(
        "deny.docker_config",
        Base::Home,
        ".docker/config.json",
        Os::Every,
    ),
    entry("deny.gnupg", Base::Home, ".gnupg", Os::Unix),
    entry(
        "deny.password_store",
        Base::Home,
        ".password-store",
        Os::Unix,
    ),
    entry("deny.netrc", Base::Home, ".netrc", Os::Every),
    entry(
        "deny.git_credentials",
        Base::Home,
        ".git-credentials",
        Os::Every,
    ),
    entry("deny.gh", Base::Home, ".config/gh", Os::Unix),
    entry("deny.gh_windows", Base::AppData, "GitHub CLI", Os::Windows),
    entry("deny.npmrc", Base::Home, ".npmrc", Os::Every),
    entry("deny.pypirc", Base::Home, ".pypirc", Os::Every),
    entry(
        "deny.cargo_credentials",
        Base::Home,
        ".cargo/credentials.toml",
        Os::Every,
    ),
    entry(
        "deny.terraform_credentials",
        Base::Home,
        ".terraform.d/credentials.tfrc.json",
        Os::Every,
    ),
    entry("deny.vault_token", Base::Home, ".vault-token", Os::Every),
    entry("deny.env_file", Base::Anywhere, ".env", Os::Every),
    // Keychains.
    entry("deny.keychains", Base::Home, "Library/Keychains", Os::MacOs),
    entry(
        "deny.credentials_roaming",
        Base::AppData,
        "Microsoft/Credentials",
        Os::Windows,
    ),
    entry(
        "deny.credentials_local",
        Base::LocalAppData,
        "Microsoft/Credentials",
        Os::Windows,
    ),
    entry(
        "deny.protect",
        Base::AppData,
        "Microsoft/Protect",
        Os::Windows,
    ),
    // Browser profiles of every browser ADR 0056 reads: the Chromium family,
    // Firefox and Safari.
    entry(
        "deny.chrome",
        Base::Home,
        ".config/google-chrome",
        Os::Linux,
    ),
    entry(
        "deny.chrome_beta",
        Base::Home,
        ".config/google-chrome-beta",
        Os::Linux,
    ),
    entry(
        "deny.chrome_unstable",
        Base::Home,
        ".config/google-chrome-unstable",
        Os::Linux,
    ),
    entry("deny.chromium", Base::Home, ".config/chromium", Os::Linux),
    entry("deny.chromium_snap", Base::Home, "snap/chromium", Os::Linux),
    entry("deny.edge", Base::Home, ".config/microsoft-edge", Os::Linux),
    entry("deny.brave", Base::Home, ".config/BraveSoftware", Os::Linux),
    entry("deny.vivaldi", Base::Home, ".config/vivaldi", Os::Linux),
    entry("deny.opera", Base::Home, ".config/opera", Os::Linux),
    entry("deny.firefox", Base::Home, ".mozilla", Os::Linux),
    entry("deny.firefox_snap", Base::Home, "snap/firefox", Os::Linux),
    entry(
        "deny.chrome_macos",
        Base::Home,
        "Library/Application Support/Google",
        Os::MacOs,
    ),
    entry(
        "deny.chromium_macos",
        Base::Home,
        "Library/Application Support/Chromium",
        Os::MacOs,
    ),
    entry(
        "deny.edge_macos",
        Base::Home,
        "Library/Application Support/Microsoft Edge",
        Os::MacOs,
    ),
    entry(
        "deny.brave_macos",
        Base::Home,
        "Library/Application Support/BraveSoftware",
        Os::MacOs,
    ),
    entry(
        "deny.vivaldi_macos",
        Base::Home,
        "Library/Application Support/Vivaldi",
        Os::MacOs,
    ),
    entry(
        "deny.opera_macos",
        Base::Home,
        "Library/Application Support/com.operasoftware.Opera",
        Os::MacOs,
    ),
    entry(
        "deny.firefox_macos",
        Base::Home,
        "Library/Application Support/Firefox",
        Os::MacOs,
    ),
    entry("deny.safari", Base::Home, "Library/Safari", Os::MacOs),
    entry(
        "deny.cookies_macos",
        Base::Home,
        "Library/Cookies",
        Os::MacOs,
    ),
    entry(
        "deny.chrome_windows",
        Base::LocalAppData,
        "Google/Chrome",
        Os::Windows,
    ),
    entry(
        "deny.chromium_windows",
        Base::LocalAppData,
        "Chromium",
        Os::Windows,
    ),
    entry(
        "deny.edge_windows",
        Base::LocalAppData,
        "Microsoft/Edge",
        Os::Windows,
    ),
    entry(
        "deny.brave_windows",
        Base::LocalAppData,
        "BraveSoftware",
        Os::Windows,
    ),
    entry(
        "deny.vivaldi_windows",
        Base::LocalAppData,
        "Vivaldi",
        Os::Windows,
    ),
    entry(
        "deny.opera_windows",
        Base::AppData,
        "Opera Software",
        Os::Windows,
    ),
    entry(
        "deny.firefox_windows",
        Base::AppData,
        "Mozilla",
        Os::Windows,
    ),
    // Shell startup files.
    entry("deny.bashrc", Base::Home, ".bashrc", Os::Unix),
    entry("deny.bash_profile", Base::Home, ".bash_profile", Os::Unix),
    entry("deny.zshrc", Base::Home, ".zshrc", Os::Unix),
    entry("deny.zprofile", Base::Home, ".zprofile", Os::Unix),
    entry("deny.profile", Base::Home, ".profile", Os::Unix),
    entry(
        "deny.powershell",
        Base::Home,
        ".config/powershell",
        Os::Unix,
    ),
    entry(
        "deny.powershell_windows",
        Base::Home,
        "Documents/PowerShell",
        Os::Windows,
    ),
    entry(
        "deny.windows_powershell",
        Base::Home,
        "Documents/WindowsPowerShell",
        Os::Windows,
    ),
    // Version-control hooks.
    entry("deny.git_hooks", Base::Anywhere, ".git/hooks", Os::Every),
];

#[cfg(test)]
mod tests;
