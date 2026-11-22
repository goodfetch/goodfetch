//! A live Run's Secrets and its data key (ADR 0145): what it resolves its
//! Secret references through, holding every value it resolved, and the
//! key that seals and opens the fields of its secret columns, made at its
//! first seal or unwrapped once per Run, stored only wrapped by the
//! key-encryption key its settings name (ADR 0015).

use std::sync::Arc;

use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::{
    KEY_REFERENCE_IN_KEYCHAIN, RUN_STATE_KEY_REFERENCE, SecretFailure,
};
use goodfetch_protocol::settings::SettingsLayer;
use secrecy::SecretString;
use secrecy::zeroize::Zeroizing;

use super::RunKeyError;
use crate::run_identity::RunIdentity;
use crate::run_key::{self, KeyError, RunKey, Sealed, StoredKey};
use crate::run_state::{RunState, SecretColumn};
use crate::secrets::{Purpose, RunSecrets, SecretError, Secrets, is_engine_key};
use crate::settings::{RunSettings, Settings};

/// A live Run's Secrets and its data key, which a function takes in place
/// of the Run's handle (ADR 0145). Clones share them.
#[derive(Clone)]
pub struct RunKeys(Arc<Inner>);

struct Inner {
    /// What it resolves its Secret references through.
    secrets: RunSecrets,
    /// Its data key, once made or unwrapped.
    key: tokio::sync::OnceCell<RunKey>,
    /// Its Run ID and settings, which name its key-encryption key.
    identity: RunIdentity,
    /// Its run state's writer, which keeps the wrapped key.
    state: RunState,
    /// The engine's settings, whose host default names the key-encryption
    /// key where no layer of the Run's settings does.
    settings: Settings,
    /// The engine's Secrets, which make the engine's own key where it is
    /// missing.
    engine_secrets: Secrets,
}

/// What a live Run's Secrets and data key are made of.
pub(super) struct KeysParts {
    pub(super) secrets: RunSecrets,
    /// Its data key when its start or resume unwrapped one.
    pub(super) key: Option<RunKey>,
    pub(super) identity: RunIdentity,
    pub(super) state: RunState,
    pub(super) settings: Settings,
    pub(super) engine_secrets: Secrets,
}

impl std::fmt::Debug for RunKeys {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunKeys")
            .field("run_id", self.0.identity.id())
            .finish_non_exhaustive()
    }
}

impl RunKeys {
    /// The Secrets and data key `parts` make.
    pub(super) fn new(parts: KeysParts) -> Self {
        Self(Arc::new(Inner {
            secrets: parts.secrets,
            key: tokio::sync::OnceCell::new_with(parts.key),
            identity: parts.identity,
            state: parts.state,
            settings: parts.settings,
            engine_secrets: parts.engine_secrets,
        }))
    }

    /// What the Run resolves its Secret references through, holding every
    /// value it resolved.
    pub fn secrets(&self) -> &RunSecrets {
        &self.0.secrets
    }

    /// Whether the Run holds its data key, made or unwrapped.
    pub(super) fn holds_key(&self) -> bool {
        self.0.key.get().is_some()
    }

    /// `plaintext` sealed under its data key for the row `row` of the secret
    /// column `column`: the only value its owner writes there. Its first
    /// call makes the data key.
    ///
    /// # Errors
    ///
    /// As [`RunKeyError`] says.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future of the caller that makes the key abandons the
    /// making, and the next caller makes one again. A drop once the
    /// wrapped key's write was queued may leave it stored: the next making
    /// then takes that key rather than store another.
    pub async fn seal(
        &self,
        column: &SecretColumn,
        row: &str,
        plaintext: &[u8],
    ) -> Result<Sealed, RunKeyError> {
        let key = self.key().await?;
        Ok(Sealed::new(key.seal(
            column.table,
            column.column,
            row,
            plaintext,
        )?))
    }

    /// The plaintext of `sealed`, read from the row `row` of the secret
    /// column `column`, in this Run or after its resume. It never makes a
    /// data key: a Run with none held, unwrapped at its resume or stored
    /// by a making whose caller was dropped, sealed nothing, so nothing it
    /// holds opens.
    ///
    /// # Errors
    ///
    /// `Key` when the value was sealed for another place or another Run,
    /// or changed since, or when the Run has no data key.
    ///
    /// # Cancel safety
    ///
    /// Dropping its future abandons the read of the stored key, which
    /// writes nothing; the next call reads it again.
    pub async fn open(
        &self,
        column: &SecretColumn,
        row: &str,
        sealed: &Sealed,
    ) -> Result<Zeroizing<Vec<u8>>, RunKeyError> {
        let key = self.0.key.get_or_try_init(|| self.stored_key()).await?;
        Ok(key.open(column.table, column.column, row, sealed.as_bytes())?)
    }

    /// Its Run ID.
    fn id(&self) -> &RunId {
        self.0.identity.id()
    }

    /// Its data key, which seals and opens the fields of its secret
    /// columns, made at its first use.
    async fn key(&self) -> Result<&RunKey, RunKeyError> {
        self.0.key.get_or_try_init(|| self.make_key()).await
    }

    /// The data key its run state holds wrapped, which a making whose
    /// caller was dropped stored without the Run taking it, unwrapped by
    /// the key-encryption key its settings name; it never makes one, nor
    /// the engine's default key-encryption key.
    async fn stored_key(&self) -> Result<RunKey, RunKeyError> {
        let stored = self
            .0
            .state
            .read(run_key::stored)
            .await?
            .ok_or(KeyError::DoesNotOpen)?;
        let kek = if let Some(kek) = self.secrets().held_for(RUN_STATE_KEY_REFERENCE.name).await {
            kek
        } else {
            let (reference, layer) =
                key_reference(&self.0.settings, self.0.identity.settings()).await;
            key_encryption_key(
                &self.0.engine_secrets,
                self.secrets(),
                &reference,
                layer,
                self.id(),
                Making::No,
            )
            .await?
        };
        Ok(RunKey::unwrap(self.id().clone(), &stored.wrapped, &kek)?)
    }

    /// Makes its data key, stored only wrapped by the key-encryption key
    /// its settings name, which resolves now unless its start resolved it;
    /// the engine's own default key is made where it is missing (#616).
    async fn make_key(&self) -> Result<RunKey, RunKeyError> {
        let (reference, layer) = key_reference(&self.0.settings, self.0.identity.settings()).await;
        let kek = match self.secrets().held_for(RUN_STATE_KEY_REFERENCE.name).await {
            Some(kek) => kek,
            None => {
                key_encryption_key(
                    &self.0.engine_secrets,
                    self.secrets(),
                    &reference,
                    layer,
                    self.id(),
                    Making::Yes,
                )
                .await?
            }
        };
        let key = RunKey::generate(self.id().clone())?;
        let stored = StoredKey {
            wrapped: key.wrap(&kek)?,
            key_reference: reference.to_string(),
        };
        // A making whose caller was dropped once its write was queued may
        // have stored a key the Run never took: that one is the Run's key,
        // and this making's is not stored (the check and the write are one
        // job of the writer, so no second key can slip between them).
        let earlier = self
            .0
            .state
            .write(move |connection| match run_key::stored(connection)? {
                Some(earlier) => Ok(Some(earlier)),
                None => run_key::store(connection, &stored).map(|()| None),
            })
            .await?;
        match earlier {
            Some(earlier) => Ok(RunKey::unwrap(self.id().clone(), &earlier.wrapped, &kek)?),
            None => Ok(key),
        }
    }
}

/// Whether a resolution of the key-encryption key makes the engine's own
/// key where it is missing: a Run's first seal does, a resume never, as a
/// new key would open no data key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Making {
    Yes,
    No,
}

/// The Secret reference of the key-encryption key of a Run whose settings
/// resolved as `run_settings`, and the layer that set it: the host's
/// default in the engine's `settings` where none did, or where a layer
/// cleared it, asked for here, so the OS keychain opens only once a Run
/// first needs its key, not as its settings open (issue 2204).
pub(super) async fn key_reference(
    settings: &Settings,
    run_settings: Option<&RunSettings>,
) -> (SecretReference, SettingsLayer) {
    if let Some((reference, layer)) = layer_set_key_reference(run_settings) {
        return (reference, layer);
    }
    let default = settings.host_defaults().await.of(&RUN_STATE_KEY_REFERENCE);
    let reference = default
        .as_str()
        .and_then(|text| SecretReference::parse(text).ok())
        .unwrap_or_else(|| {
            SecretReference::parse(KEY_REFERENCE_IN_KEYCHAIN)
                .expect("invariant: the keychain's default is a Secret reference")
        });
    (reference, SettingsLayer::Default)
}

/// The key-encryption key `reference`, set by `layer`, gives for the Run
/// `run_id`, resolved through `secrets`. With `Making::Yes`, the engine's
/// own key ([`is_engine_key`]) is made first, through `engine_secrets`,
/// where it is missing.
pub(super) async fn key_encryption_key(
    engine_secrets: &Secrets,
    secrets: &RunSecrets,
    reference: &SecretReference,
    layer: SettingsLayer,
    run_id: &RunId,
    making: Making,
) -> Result<SecretString, SecretError> {
    let purpose = Purpose {
        placeholder: RUN_STATE_KEY_REFERENCE.name,
        domain: None,
        domains: &[],
        layer: Some(layer),
        run_id: Some(run_id),
    };
    match secrets.resolve(reference, &purpose).await {
        Err(SecretError::Unavailable {
            failure: SecretFailure::Missing | SecretFailure::NotConfigured,
            ..
        }) if making == Making::Yes && is_engine_key(reference) => {
            engine_secrets.make_key(reference).await?;
            secrets.resolve(reference, &purpose).await
        }
        resolved => resolved,
    }
}

/// The Secret reference of the key-encryption key that a layer of
/// `settings` set, and that layer; none where the default stands.
pub(super) fn layer_set_key_reference(
    settings: Option<&RunSettings>,
) -> Option<(SecretReference, SettingsLayer)> {
    let resolved = settings?.resolved();
    let layer = resolved.effective(&RUN_STATE_KEY_REFERENCE)?.layer;
    (layer != SettingsLayer::Default).then_some(())?;
    Some((
        resolved.get::<Option<SecretReference>>(&RUN_STATE_KEY_REFERENCE)?,
        layer,
    ))
}
