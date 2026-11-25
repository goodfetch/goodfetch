//! What the cloud providers' ambient credential sources share (issue
//! 1658): how they reach their identity endpoints, the wait a metadata
//! service gets before the host is taken to have none, the files they
//! read, and the failures they answer.

use std::path::Path;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::secret_endpoint::SecretProviderEndpoint;
use goodfetch_engine::secrets::{ConfigProblem, PartyText, ProviderFailure};
use goodfetch_engine::tasks::BlockingThreads;
use secrecy::SecretString;
use serde_json::Value;

/// How long a cloud's metadata service may take to answer before the host
/// is taken to have none, as the clouds' own SDKs wait.
pub(crate) const METADATA_WAIT: Duration = Duration::from_secs(1);

/// What the sources reach their endpoints and files through.
pub(crate) struct Reach<'a> {
    pub(crate) endpoint: &'a SecretProviderEndpoint,
    pub(crate) blocking: &'a BlockingThreads,
    pub(crate) clock: Clock,
}

impl Reach<'_> {
    /// `asking`'s answer, or none when it takes longer than
    /// [`METADATA_WAIT`].
    pub(crate) async fn promptly<T>(&self, asking: impl Future<Output = T>) -> Option<T> {
        tokio::select! {
            answer = asking => Some(answer),
            () = self.clock.sleep(METADATA_WAIT) => None,
        }
    }

    /// The text of the file at `path`, without the line ending a platform
    /// may leave, read on the blocking threads.
    #[expect(
        clippy::disallowed_methods,
        reason = "the credential file is read on the Secrets' owner's blocking threads, never on an async worker"
    )]
    pub(crate) async fn read_file(&self, path: &Path) -> std::io::Result<SecretString> {
        let path = path.to_owned();
        self.blocking
            .run(move || std::fs::read_to_string(path))
            .await
            .map(|text| SecretString::from(text.trim().to_owned()))
    }
}

pub(crate) fn not_configured(fix: String) -> ProviderFailure {
    ProviderFailure::NotConfigured(ConfigProblem { fix })
}

/// An answer of `service`'s that is none of the shapes it documents, kept
/// as `what` it lacked: the answer itself is never kept, since it may hold
/// credentials.
pub(crate) fn unrecognized(service: &str, what: &str) -> ProviderFailure {
    tracing::warn!(service, "an identity endpoint answered an unknown shape");
    ProviderFailure::Unrecognized(PartyText::new(format!("{service} answered {what}")))
}

/// The `access_token` of an OAuth 2.0 token answer, `body`, of `service`'s.
pub(crate) fn access_token(body: &[u8], service: &str) -> Result<SecretString, ProviderFailure> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|answer| {
            answer
                .get("access_token")
                .and_then(Value::as_str)
                .map(|token| SecretString::from(token.to_owned()))
        })
        .ok_or_else(|| unrecognized(service, "no `access_token`"))
}
