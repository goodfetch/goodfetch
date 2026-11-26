//! The `email` channel (ADR 0027; issue 1388): a Notification rendered as
//! one message of two parts, plain text and a minimal HTML, its link
//! without the user and password it may carry, sent through the channel's
//! provider: an SMTP server ([`Smtp`]) or Amazon SES ([`Ses`]), which take
//! the rendered message raw, or the HTTP APIs of Resend ([`Resend`]),
//! Postmark ([`Postmark`]), SendGrid ([`Sendgrid`]) and Mailgun
//! ([`Mailgun`]) (issue 1389), which take its subject and parts. Each
//! provider reaches its server through the engine's Notification channel
//! endpoint, its own client and never a Session's (ADR 0088), and is a
//! translation point of its own (ADR 0127), mapping its replies to the
//! driver's [`Failure`].

mod mailgun;
mod postmark;
mod render;
mod resend;
mod sendgrid;
mod ses;
mod smtp;

use std::time::Duration;

use goodfetch_engine::clock::Clock;

use crate::api::{self, ApiFailure};
use crate::driver::Channel;
use crate::failure::Failure;
use crate::message::Message;

pub use mailgun::Mailgun;
pub use postmark::Postmark;
pub use resend::Resend;
pub use sendgrid::Sendgrid;
pub use ses::{RegionUnusable, Ses, SesFailure};
pub use smtp::{Login, Smtp, SmtpFailure};

/// How long a delivery waits for its provider to take the message before
/// it counts as [`Failure::Timeout`].
pub const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// An email address, `local@domain`, as an SMTP envelope and a message's
/// header carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address(String);

/// Why a text is no email address the channel sends from or to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "an email address is `local@domain`, with no space, control character, `<`, `>` or `,`, such as `ops@example.com`"
)]
#[non_exhaustive]
pub struct AddressUnusable;

impl Address {
    /// Reads `text` as an address: a non-empty local part and domain on
    /// either side of its last `@`, with no space, control character, `<`,
    /// `>` or `,`, any of which would break the envelope or a header.
    ///
    /// # Errors
    ///
    /// [`AddressUnusable`] for any other text.
    pub fn parse(text: &str) -> Result<Self, AddressUnusable> {
        let usable = |character: char| {
            !(character.is_whitespace()
                || character.is_control()
                || matches!(character, '<' | '>' | ','))
        };
        match text.rsplit_once('@') {
            Some((local, domain))
                if !local.is_empty() && !domain.is_empty() && text.chars().all(usable) =>
            {
                Ok(Self(text.to_owned()))
            }
            _ => Err(AddressUnusable),
        }
    }

    /// Its text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Its domain, after its last `@`.
    fn domain(&self) -> &str {
        self.0.rsplit_once('@').map_or("", |(_, domain)| domain)
    }
}

/// The provider an `email` channel sends through.
#[derive(Debug)]
#[non_exhaustive]
pub enum Provider {
    /// An SMTP server.
    Smtp(Smtp),
    /// Amazon SES.
    Ses(Ses),
    /// Resend.
    Resend(Resend),
    /// Postmark.
    Postmark(Postmark),
    /// SendGrid.
    Sendgrid(Sendgrid),
    /// Mailgun.
    Mailgun(Mailgun),
}

/// An `email` channel.
#[derive(Debug)]
pub struct Email {
    name: String,
    from: Address,
    to: Vec<Address>,
    provider: Provider,
    clock: Clock,
}

/// Why an `email` channel cannot be made: it has no address to send to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("an `email` channel names at least one address in `email.to`")]
#[non_exhaustive]
pub struct NoRecipient;

/// Why a delivery got no success from the provider.
#[derive(Debug)]
#[non_exhaustive]
pub enum EmailFailure {
    /// The provider did not take the message within [`SEND_TIMEOUT`].
    TimedOut,
    /// The SMTP server's failure.
    Smtp(SmtpFailure),
    /// Amazon SES's failure.
    Ses(SesFailure),
    /// Resend's failure.
    Resend(ApiFailure),
    /// Postmark's failure.
    Postmark(ApiFailure),
    /// SendGrid's failure.
    Sendgrid(ApiFailure),
    /// Mailgun's failure.
    Mailgun(ApiFailure),
}

impl Email {
    /// The channel `name` sending from `from` to each of `to` through
    /// `provider`.
    ///
    /// # Errors
    ///
    /// [`NoRecipient`] when `to` is empty.
    pub fn new(
        name: impl Into<String>,
        from: Address,
        to: Vec<Address>,
        provider: Provider,
        clock: Clock,
    ) -> Result<Self, NoRecipient> {
        if to.is_empty() {
            return Err(NoRecipient);
        }
        Ok(Self {
            name: name.into(),
            from,
            to,
            provider,
            clock,
        })
    }
}

impl Channel for Email {
    type Failure = EmailFailure;

    fn name(&self) -> &str {
        &self.name
    }

    fn destination(&self) -> &str {
        match &self.provider {
            Provider::Smtp(smtp) => smtp.host(),
            Provider::Ses(ses) => ses.host(),
            Provider::Resend(resend) => resend.host(),
            Provider::Postmark(postmark) => postmark.host(),
            Provider::Sendgrid(sendgrid) => sendgrid.host(),
            Provider::Mailgun(mailgun) => mailgun.host(),
        }
    }

    async fn send(&self, message: &Message) -> Result<(), EmailFailure> {
        let raw = || render::message(message, &self.from, &self.to, self.clock.wall());
        let (from, to, clock) = (&self.from, self.to.as_slice(), self.clock);
        let sent = async {
            match &self.provider {
                Provider::Smtp(smtp) => smtp
                    .send(from, to, &raw())
                    .await
                    .map_err(EmailFailure::Smtp),
                Provider::Ses(ses) => ses
                    .send(from, to, raw(), clock)
                    .await
                    .map_err(EmailFailure::Ses),
                Provider::Resend(resend) => resend
                    .send(
                        from,
                        to,
                        &render::parts(message),
                        &self.name,
                        &message.event_id,
                        clock,
                    )
                    .await
                    .map_err(EmailFailure::Resend),
                Provider::Postmark(postmark) => postmark
                    .send(from, to, &render::parts(message), clock)
                    .await
                    .map_err(EmailFailure::Postmark),
                Provider::Sendgrid(sendgrid) => sendgrid
                    .send(from, to, &render::parts(message), clock)
                    .await
                    .map_err(EmailFailure::Sendgrid),
                Provider::Mailgun(mailgun) => mailgun
                    .send(from, to, &render::parts(message), clock)
                    .await
                    .map_err(EmailFailure::Mailgun),
            }
        };
        tokio::select! {
            sent = sent => sent,
            () = self.clock.sleep(SEND_TIMEOUT) => Err(EmailFailure::TimedOut),
        }
    }

    fn map_failure(&self, failure: EmailFailure) -> Failure {
        match failure {
            EmailFailure::TimedOut => Failure::Timeout,
            EmailFailure::Smtp(failure) => smtp::translate(failure),
            EmailFailure::Ses(failure) => ses::translate(failure),
            EmailFailure::Resend(failure) => api::translate(failure, &resend::TABLE),
            EmailFailure::Postmark(failure) => api::translate(failure, &postmark::TABLE),
            EmailFailure::Sendgrid(failure) => api::translate(failure, &sendgrid::TABLE),
            EmailFailure::Mailgun(failure) => api::translate(failure, &mailgun::TABLE),
        }
    }
}
