//! A Connection's Saved identities for a program that does not use async (ADR
//! 0081): the twin of [`crate::Identities`], whose calls end in `send()`.

use goodfetch_protocol::page::identity::{
    IdentityDeleteParams, IdentityDeleted, IdentityExportParams, IdentityExported,
    IdentityImportParams, IdentityListParams, SavedIdentityInfo,
};

use super::{Call, ClientConnection};

impl ClientConnection {
    /// The Connection's Saved identities, as
    /// [`crate::ClientConnection::identities`]'s, reached without I/O.
    #[must_use]
    pub fn identities(&self) -> Identities {
        Identities(self.0.identities(), self.0.clone())
    }
}

/// A Connection's Saved identities, whose calls block: the twin of
/// [`crate::Identities`]. Cloning it is cheap.
#[derive(Clone, Debug)]
pub struct Identities(crate::Identities, crate::ClientConnection);

impl Identities {
    /// Lists every non-expired Saved identity in this Product major's store,
    /// with its metadata and domains, never secret values (ADR 0035).
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn list(&self) -> Call<Vec<SavedIdentityInfo>, IdentityListParams> {
        Call::new(self.0.list(), &self.1)
    }

    /// Imports cookies and storage from a Session source (`state_file`,
    /// `cookie_header`, etc.) into the encrypted per-major Saved identity
    /// store under `name`, replacing any existing identity of that name under
    /// the install-wide lock and recording the import in the Audit log
    /// (ADRs 0035 and 0056).
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn import(
        &self,
        params: IdentityImportParams,
    ) -> Call<SavedIdentityInfo, IdentityImportParams> {
        Call::new(self.0.import(params), &self.1)
    }

    /// Deletes the named Saved identity from this Product major's encrypted
    /// store, wiping its secret fields and recording the deletion in the
    /// Audit log (ADR 0035).
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn delete(&self, name: &str) -> Call<IdentityDeleted, IdentityDeleteParams> {
        Call::new(self.0.delete(name), &self.1)
    }

    /// Exports the named Saved identity as an `age`-encrypted `session-export`
    /// bundle to an `age` public key or a passphrase Secret reference,
    /// preserving its Access mode, Identity profile seed, proxy binding and
    /// machine binding, and recording the export in the Audit log (ADRs 0015,
    /// 0035 and 0056).
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn export(
        &self,
        params: IdentityExportParams,
    ) -> Call<IdentityExported, IdentityExportParams> {
        Call::new(self.0.export(params), &self.1)
    }
}
