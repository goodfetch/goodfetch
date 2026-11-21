//! The `signing_key.*` operations every Product carries (ADRs 0045 and
//! 0056; #1551): `signing_key.create` makes an `identified` Session's
//! Ed25519 key in the OS keychain or an encrypted local file, and
//! `signing_key.directory` exports the key directory of keys already kept
//! there, which the user serves at the origin `identified.signature_agent`
//! names. Both resolve each key outside any Run through the engine's
//! providers, recorded in the Audit log as `identified.signing_key`'s use,
//! and neither answers nor quotes a private key.

use std::time::{Duration, SystemTime};

use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::access_mode::IDENTIFIED_SIGNING_KEY;
use goodfetch_protocol::page::signing_key::{
    DEFAULT_VALID_FOR, DIRECTORY_CONTENT_TYPE, DIRECTORY_PATH, PublicSigningKey,
    SIGNING_KEY_CREATE, SIGNING_KEY_DIRECTORY, SigningKeyCreateParams, SigningKeyCreated,
    SigningKeyDirectory, SigningKeyDirectoryParams,
};
use goodfetch_protocol::secret_reference::SecretReference;
use secrecy::{ExposeSecret as _, SecretString};

use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::request_signing::{self, DirectoryError};
use crate::run::secret_error;
use crate::secrets::{Purpose, RunSecrets, can_hold_a_signing_key};

/// The shortest time an exported directory's signatures stay valid, in
/// milliseconds, as `valid_for`'s schema bounds it.
const SHORTEST_VALID_FOR: u64 = 1000;

/// The `signing_key.*` handlers.
pub(crate) fn handlers() -> [Handler; 2] {
    [
        Handler::new(&SIGNING_KEY_CREATE, create),
        Handler::new(&SIGNING_KEY_DIRECTORY, directory),
    ]
}

async fn create(
    params: SigningKeyCreateParams,
    context: CallContext,
) -> Result<SigningKeyCreated, CallError> {
    let reference = parsed(&params.signing_key)?;
    if !can_hold_a_signing_key(&reference) {
        return Err(invalid(format!(
            "`{reference}` names no item the engine can keep a signing key in: name a `keychain:` or `secretfile:` item of your own"
        )));
    }
    let candidate = request_signing::new_key()
        .map_err(|error| CallError::protocol(ProtocolFailure::InternalError, error.to_string()))?;
    let (secrets, resolving) = context.runs().outside_a_run();
    secrets
        .make_signing_key(&reference, &candidate)
        .await
        .map_err(|error| secret_error(&error))?;
    let kept = resolved(&resolving, &reference).await?;
    let public = request_signing::public_key(&kept).map_err(|error| {
        invalid(format!(
            "`{reference}` holds something other than a signing key, which is kept: {error}; name another item"
        ))
    })?;
    Ok(SigningKeyCreated {
        signing_key: reference.to_string(),
        key_id: public.kid.clone(),
        created: kept.expose_secret() == candidate.expose_secret(),
        public_key: PublicSigningKey {
            kty: public.kty.to_owned(),
            crv: public.crv.to_owned(),
            x: public.x,
            kid: public.kid,
        },
    })
}

async fn directory(
    params: SigningKeyDirectoryParams,
    context: CallContext,
) -> Result<SigningKeyDirectory, CallError> {
    let valid_for = params.valid_for.unwrap_or(DEFAULT_VALID_FOR);
    if valid_for < SHORTEST_VALID_FOR {
        return Err(invalid(format!(
            "`valid_for` is {valid_for} ms, under the shortest, {SHORTEST_VALID_FOR} ms"
        )));
    }
    let references = params
        .signing_keys
        .iter()
        .map(|text| parsed(text))
        .collect::<Result<Vec<_>, _>>()?;
    let (_, resolving) = context.runs().outside_a_run();
    let mut keys = Vec::with_capacity(references.len());
    for reference in &references {
        keys.push(resolved(&resolving, reference).await?);
    }
    let exported =
        request_signing::directory(&keys, &params.authority, Duration::from_millis(valid_for))
            .map_err(|error| match error {
                DirectoryError::Key { index, error } => invalid(format!(
                    "`{}` gives no signing key: {error}",
                    references[index]
                )),
                DirectoryError::NoRandomness(_) => {
                    CallError::protocol(ProtocolFailure::InternalError, error.to_string())
                }
                DirectoryError::NoKeys | DirectoryError::BadAuthority(_) => {
                    invalid(error.to_string())
                }
            })?;
    let expires = jiff::Timestamp::from_second(exported.expires)
        .expect("invariant: a signature made now expires within jiff's range");
    Ok(SigningKeyDirectory {
        path: DIRECTORY_PATH.to_owned(),
        content_type: DIRECTORY_CONTENT_TYPE.to_owned(),
        body: exported.body,
        signature_input: exported.signature_input,
        signature: exported.signature,
        key_ids: exported.key_ids,
        expires: SystemTime::from(expires),
    })
}

/// `text` as a Secret reference, or `invalid_params`.
fn parsed(text: &str) -> Result<SecretReference, CallError> {
    SecretReference::parse(text)
        .map_err(|error| invalid(format!("`{text}` is no Secret reference: {error}")))
}

/// The key `reference` gives, resolved now for `identified.signing_key`.
async fn resolved(
    resolving: &RunSecrets,
    reference: &SecretReference,
) -> Result<SecretString, CallError> {
    resolving
        .resolve(
            reference,
            &Purpose {
                placeholder: IDENTIFIED_SIGNING_KEY.name,
                domain: None,
                domains: &[],
                layer: None,
                run_id: None,
            },
        )
        .await
        .map_err(|error| secret_error(&error))
}

/// `invalid_params` with `message`.
fn invalid(message: String) -> CallError {
    CallError::protocol(ProtocolFailure::InvalidParams, message)
}
