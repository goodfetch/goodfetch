//! The running Product (ADRs 0087, 0090 and 0095): which Product the engine
//! runs as, its Product version, the channel it was installed from, and the
//! refusal of what that Product lacks, `not_in_product`, whose `fix` names
//! the install command of the Product that carries the request on that
//! channel, or, behind Goodfetch Cloud's gateway, ADR 0096's wording.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{NotInProductDetails, ProductVersion};

/// The channel a Product's executable was installed from (ADR 0090).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Channel {
    /// npm.
    Npm,
    /// The Python Package Index.
    Pypi,
    /// crates.io, through `cargo install`, which builds from source.
    CratesIo,
    /// Goodfetch's Homebrew tap.
    Homebrew,
    /// The OCI image.
    Oci,
    /// The `.sh` or `.ps1` installer.
    Installer,
}

impl Channel {
    /// Every channel.
    pub const ALL: [Self; 6] = [
        Self::Npm,
        Self::Pypi,
        Self::CratesIo,
        Self::Homebrew,
        Self::Oci,
        Self::Installer,
    ];

    /// Its name, as `GOODFETCH_CHANNEL` and a release build's stamp write
    /// it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pypi => "pypi",
            Self::CratesIo => "crates-io",
            Self::Homebrew => "homebrew",
            Self::Oci => "oci",
            Self::Installer => "installer",
        }
    }

    /// The channel named `name`, if one is.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|channel| channel.name() == name)
    }

    /// The command that installs `product`'s executable on this channel.
    pub fn install_command(self, product: Product) -> String {
        let name = product.as_str();
        match self {
            Self::Npm => format!("npm install @goodfetch/{name}"),
            Self::Pypi => format!("pip install goodfetch-{name}"),
            Self::CratesIo => {
                format!("cargo install goodfetch-{name} --locked --features executable")
            }
            Self::Homebrew => format!("brew install goodfetch/tap/goodfetch-{name}"),
            Self::Oci => format!("docker pull ghcr.io/goodfetch/{name}"),
            Self::Installer if cfg!(windows) => {
                format!("irm https://goodfetch.dev/install/{name}.ps1 | iex")
            }
            Self::Installer => format!("curl -fsSL https://goodfetch.dev/install/{name}.sh | sh"),
        }
    }
}

/// What the engine runs as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// Its Product.
    pub product: Product,
    /// Its Product version.
    pub product_version: ProductVersion,
    /// The channel its executable came from.
    pub channel: Channel,
}

/// Where a request for what the Product lacks came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requested<'a> {
    /// A Client's `hello` that needs the Product.
    Hello,
    /// A call of the operation, setting or value named.
    Name(&'a str),
}

impl Identity {
    /// The message and details of `not_in_product` for a request that
    /// `needs` carries and this engine's Product lacks; `behind_gateway`
    /// when the Connection's `hello` came from Goodfetch Cloud's gateway
    /// (ADR 0096).
    pub fn not_in_product(
        &self,
        requested: Requested<'_>,
        needs: Product,
        behind_gateway: bool,
    ) -> (String, NotInProductDetails) {
        let (engine, version, needed) = (self.product.name(), &self.product_version, needs.name());
        let install = self.channel.install_command(needs);
        let (message, fix) = match (requested, behind_gateway) {
            (Requested::Hello, false) => (
                format!("This Client needs {needed}, and this engine is {engine} {version}."),
                format!(
                    "Connect to a {needed} engine, or run with {needed}, which contains everything {engine} does (`{install}`)."
                ),
            ),
            (Requested::Name(name), false) => (
                format!("`{name}` is part of {needed}, and this engine is {engine} {version}."),
                format!(
                    "Run it with {needed}, which contains everything {engine} does (`{install}`), or connect to a {needed} engine."
                ),
            ),
            (Requested::Hello, true) => (
                format!(
                    "Goodfetch Cloud's gateway asked for {needed} on a run of {engine}'s engine."
                ),
                format!("Only a run started for {needed} carries it; start the run for {needed}."),
            ),
            (Requested::Name(name), true) => (
                format!("`{name}` is part of {needed}, and this run is {engine}'s."),
                format!(
                    "Only a run started for {needed} carries `{name}`; start the run for {needed}."
                ),
            ),
        };
        let details = NotInProductDetails {
            product: self.product,
            product_version: self.product_version.clone(),
            needs,
            requested: match requested {
                Requested::Hello => None,
                Requested::Name(name) => Some(name.to_owned()),
            },
            fix,
            install: (!behind_gateway).then_some(install),
            handover: false,
        };
        (message, details)
    }
}
