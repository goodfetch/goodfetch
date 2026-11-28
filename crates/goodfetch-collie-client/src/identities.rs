//! A Connection's Saved identities (ADRs 0015, 0035, 0056 and 0099; ticket
//! #736): `connection.identities()`, reached without I/O, exposing `list`,
//! `import`, `delete`, and `export` (as ADR 0015's `age`-encrypted bundle — no
//! SDK writes clear-text `storageState`). TypeScript's `identities.ts` and
//! Python's `identities.py` are its twins.

use goodfetch_protocol::page::identity::{
    IDENTITY_DELETE, IDENTITY_EXPORT, IDENTITY_IMPORT, IDENTITY_LIST, IdentityDeleteParams,
    IdentityDeleted, IdentityExportParams, IdentityExported, IdentityImportParams, IdentityList,
    IdentityListParams, SavedIdentityInfo,
};

use crate::call::Call;
use crate::connection::ClientConnection;
use crate::host::ConnectionHost;

/// A Connection's Saved identities: `connection.identities()`. Each call acts
/// on this Product major's encrypted Saved identity store. Cloning is cheap.
#[derive(Clone, Debug)]
pub struct Identities {
    connection: ClientConnection,
}

impl ClientConnection {
    /// The Connection's Saved identities, reached without I/O (ADR 0099).
    #[must_use]
    pub fn identities(&self) -> Identities {
        Identities {
            connection: self.clone(),
        }
    }
}

impl Identities {
    /// Lists every non-expired Saved identity in this Product major's store,
    /// with its metadata and domains, never secret values (ADR 0035).
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn list(&self) -> Call<Vec<SavedIdentityInfo>, IdentityListParams> {
        crate::markers::warn_operation(IDENTITY_LIST.name);
        let call: Call<IdentityList, IdentityListParams> = self
            .host()
            .call(IDENTITY_LIST.name, IdentityListParams::default());
        call.map(|listed| listed.identities)
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
        crate::markers::warn_operation(IDENTITY_IMPORT.name);
        self.host().call(IDENTITY_IMPORT.name, params)
    }

    /// Deletes the named Saved identity from this Product major's encrypted
    /// store, wiping its secret fields and recording the deletion in the
    /// Audit log (ADR 0035).
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn delete(&self, name: &str) -> Call<IdentityDeleted, IdentityDeleteParams> {
        crate::markers::warn_operation(IDENTITY_DELETE.name);
        self.host().call(
            IDENTITY_DELETE.name,
            IdentityDeleteParams {
                name: name.to_owned(),
            },
        )
    }

    /// Exports the named Saved identity as an `age`-encrypted `session-export`
    /// bundle to an `age` public key or a passphrase Secret reference,
    /// preserving its Access mode, Identity profile seed, proxy binding and
    /// machine binding, and recording the export in the Audit log (ADRs 0015,
    /// 0035 and 0056).
    ///
    /// No SDK writes clear-text `storageState` (ADR 0056).
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn export(
        &self,
        params: IdentityExportParams,
    ) -> Call<IdentityExported, IdentityExportParams> {
        crate::markers::warn_operation(IDENTITY_EXPORT.name);
        self.host().call(IDENTITY_EXPORT.name, params)
    }

    fn host(&self) -> ConnectionHost<()> {
        ConnectionHost::new(self.connection.clone(), ())
    }
}
