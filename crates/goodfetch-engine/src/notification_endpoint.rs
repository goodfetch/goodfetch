//! A Notification webhook's endpoint (ADRs 0017, 0027, 0043 and 0088;
//! issue 1385): the URL a `webhook` channel posts to, an endpoint of the
//! Notification webhook kind that each delivery reaches through the
//! engine's own HTTP client, never a Session's, so no Session cookie goes
//! with it. The egress policy judges it as its class: an endpoint on
//! loopback or a private network is reached unless
//! `egress.no_private_endpoints` refuses it, each delivery's addresses
//! judged again before it connects. Its certificate chains to Mozilla's
//! roots or to the authorities `tls.trusted_cas` lists for its own domain.

use hyper::Request;
use hyper::body::Bytes;

use crate::clock::Clock;
use crate::egress::ConfiguredKind;
use crate::http_client::{EndpointSettings, WholeFailure};
use crate::settings::Resolved;
use crate::tasks::BlockingThreads;

/// The most bytes of a webhook's answer a delivery reads: a receiver's
/// acknowledgement, whose body the channel does not read.
pub const ANSWER_LIMIT: usize = 64 * 1024;

/// A Notification webhook's endpoint, as the machine's settings let it be
/// reached.
#[derive(Clone, Debug)]
pub struct NotificationEndpoint {
    endpoint: EndpointSettings,
}

impl NotificationEndpoint {
    /// The endpoint the machine's `settings` let a webhook reach: their
    /// `egress.no_private_endpoints`, `network.resolve` and
    /// `tls.trusted_cas`, whose files each delivery reads on `blocking`.
    pub fn of(settings: &Resolved, clock: Clock, blocking: BlockingThreads) -> Self {
        Self {
            endpoint: EndpointSettings::of(settings, clock, blocking),
        }
    }

    /// Sends `request`, whose URI is the webhook's absolute URL, and
    /// answers the webhook's whole answer.
    ///
    /// # Errors
    ///
    /// [`WholeFailure`] when the policy refused the endpoint, its
    /// certificate failed, or it gave no whole answer; the `webhook`
    /// channel bounds how long it waits.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection it opened.
    pub async fn send(
        &self,
        request: Request<Bytes>,
    ) -> Result<hyper::Response<Bytes>, WholeFailure> {
        self.endpoint
            .send_whole(ConfiguredKind::NotificationWebhook, request, ANSWER_LIMIT)
            .await
    }
}
