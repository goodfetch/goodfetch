//! The `smtp` provider: one SMTP dialogue per delivery (RFC 5321) with the
//! channel's server, over TLS from the first byte or upgraded with
//! `STARTTLS` before any login, never sent in the clear; its translation
//! point maps each reply, by its code and enhanced status code (RFC 3463),
//! to the driver's [`Failure`].

use goodfetch_engine::http_client::WholeFailure;
use goodfetch_engine::notification_endpoint::{MailConnection, NotificationEndpoint};
use goodfetch_protocol::notifications::SmtpSecurity;
use mail_send::SmtpClient;
use mail_send::smtp::message::Parameters;
use secrecy::{ExposeSecret as _, SecretString};
use smtp_proto::{EXT_START_TLS, EhloResponse, Response};

use super::{Address, SEND_TIMEOUT};
use crate::api::PARTY_TEXT_LIMIT;
use crate::failure::{ChannelText, Failure};

/// The port of SMTP over TLS from the first byte (RFC 8314).
const IMPLICIT_TLS_PORT: u16 = 465;

/// The submission port, upgraded with `STARTTLS` (RFC 6409).
const SUBMISSION_PORT: u16 = 587;

/// The user name and password an SMTP channel logs in with.
#[derive(Clone, Debug)]
pub struct Login {
    /// The user name.
    pub username: SecretString,
    /// The password.
    pub password: SecretString,
}

/// An `smtp` provider: the server and how the channel reaches it.
#[derive(Debug)]
pub struct Smtp {
    host: String,
    port: u16,
    security: SmtpSecurity,
    login: Option<Login>,
    endpoint: NotificationEndpoint,
}

/// A reply of the server's that is no success.
#[derive(Debug)]
pub struct Reply {
    /// Its code, such as `550`.
    pub code: u16,
    /// Its enhanced status code, such as `[5, 1, 1]`, or zeros when it has
    /// none.
    pub enhanced: [u8; 3],
    /// Its text.
    pub text: ChannelText,
}

/// Why the server did not take the message.
#[derive(Debug)]
#[non_exhaustive]
pub enum SmtpFailure {
    /// The endpoint refused the server, failed its certificate, or reached
    /// no address of it.
    Endpoint(WholeFailure),
    /// The connection broke.
    Broken,
    /// No reply within [`SEND_TIMEOUT`].
    TimedOut,
    /// A reply other than the one the dialogue's step expects.
    Reply(Reply),
    /// The server offered no `STARTTLS` on a channel that upgrades with
    /// it, so nothing was sent in the clear.
    NoStartTls,
    /// The channel logs in and the server offers no mechanism it can.
    NoAuthMechanism,
    /// A reply no SMTP reply is.
    Unreadable,
    /// Any other failure.
    Other(ChannelText),
}

impl Smtp {
    /// The server `host` on `port`, by default 465 over TLS and 587 with
    /// `STARTTLS`, secured as `security`, logged in to with `login` when
    /// given, reached through `endpoint`.
    pub fn new(
        host: impl Into<String>,
        port: Option<u16>,
        security: SmtpSecurity,
        login: Option<Login>,
        endpoint: NotificationEndpoint,
    ) -> Self {
        let port = port.unwrap_or(match security {
            SmtpSecurity::Tls => IMPLICIT_TLS_PORT,
            SmtpSecurity::Starttls => SUBMISSION_PORT,
        });
        Self {
            host: host.into(),
            port,
            security,
            login,
            endpoint,
        }
    }

    /// The server's host.
    pub(super) fn host(&self) -> &str {
        &self.host
    }

    /// Sends `message` from `from` to each of `to`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection; the server may or may
    /// not have taken the message.
    pub(super) async fn send(
        &self,
        from: &Address,
        to: &[Address],
        message: &[u8],
    ) -> Result<(), SmtpFailure> {
        let implicit = self.security == SmtpSecurity::Tls;
        let connection = self
            .endpoint
            .open_mail(&self.host, self.port, implicit)
            .await
            .map_err(SmtpFailure::Endpoint)?;
        let local = connection.local_name();
        let mut client = SmtpClient {
            stream: connection,
            timeout: SEND_TIMEOUT,
        };
        expect(client.read().await.map_err(SmtpFailure::of)?, 220)?;
        let mut capabilities = client.ehlo(&local).await.map_err(SmtpFailure::of)?;
        if !client.stream.is_secure() {
            if !capabilities.has_capability(EXT_START_TLS) {
                return Err(SmtpFailure::NoStartTls);
            }
            expect(
                client.cmd(b"STARTTLS\r\n").await.map_err(SmtpFailure::of)?,
                220,
            )?;
            client = secured(client).await?;
            capabilities = client.ehlo(&local).await.map_err(SmtpFailure::of)?;
        }
        if let Some(login) = &self.login {
            log_in(&mut client, login, &capabilities).await?;
        }
        let none = Parameters::new();
        client
            .mail_from(from.as_str(), &none)
            .await
            .map_err(SmtpFailure::of)?;
        for address in to {
            client
                .rcpt_to(address.as_str(), &none)
                .await
                .map_err(SmtpFailure::of)?;
        }
        client.data(message).await.map_err(SmtpFailure::of)?;
        // The server took the message: whatever it answers to `QUIT`, the
        // Notification is delivered.
        let _ = client.quit().await;
        Ok(())
    }
}

/// `client` over TLS, after the server answered `STARTTLS`.
async fn secured(
    client: SmtpClient<MailConnection>,
) -> Result<SmtpClient<MailConnection>, SmtpFailure> {
    let SmtpClient { stream, timeout } = client;
    let stream = stream.start_tls().await.map_err(SmtpFailure::Endpoint)?;
    Ok(SmtpClient { stream, timeout })
}

/// Logs in to the server with `login`, by the strongest mechanism the
/// server's `capabilities` offer.
async fn log_in(
    client: &mut SmtpClient<MailConnection>,
    login: &Login,
    capabilities: &EhloResponse<String>,
) -> Result<(), SmtpFailure> {
    let credentials = mail_send::Credentials::new(
        login.username.expose_secret(),
        login.password.expose_secret(),
    );
    client
        .authenticate(&credentials, capabilities)
        .await
        .map(|_| ())
        .map_err(SmtpFailure::of)
}

/// Whether `reply` has `code`, or the failure it is.
fn expect(reply: Response<String>, code: u16) -> Result<(), SmtpFailure> {
    if reply.code == code {
        Ok(())
    } else {
        Err(SmtpFailure::Reply(Reply::of(reply)))
    }
}

impl Reply {
    /// The reply `response` is, its text cut to its start.
    fn of(response: Response<String>) -> Self {
        let mut text = response.message;
        if text.len() > PARTY_TEXT_LIMIT {
            let mut end = PARTY_TEXT_LIMIT;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        Self {
            code: response.code,
            enhanced: response.esc,
            text: ChannelText::new(text),
        }
    }
}

impl SmtpFailure {
    /// The failure an SMTP client's `error` is.
    fn of(error: mail_send::Error) -> Self {
        match error {
            mail_send::Error::Io(_) => Self::Broken,
            mail_send::Error::Timeout => Self::TimedOut,
            mail_send::Error::UnexpectedReply(reply)
            | mail_send::Error::AuthenticationFailed(reply) => Self::Reply(Reply::of(reply)),
            mail_send::Error::UnparseableReply => Self::Unreadable,
            mail_send::Error::MissingStartTls => Self::NoStartTls,
            mail_send::Error::UnsupportedAuthMechanism => Self::NoAuthMechanism,
            other => Self::Other(ChannelText::new(other.to_string())),
        }
    }
}

/// The SMTP translation point (ADR 0127): the driver's failure for each
/// reply and failure it knows, and [`Failure::Unrecognized`], logged at
/// `warn` with its text, for any other.
pub(super) fn translate(failure: SmtpFailure) -> Failure {
    match failure {
        SmtpFailure::Endpoint(
            WholeFailure::CertificateInvalid(_)
            | WholeFailure::ClientCertificate(_)
            | WholeFailure::Network,
        )
        | SmtpFailure::Broken => Failure::Unreachable,
        SmtpFailure::TimedOut => Failure::Timeout,
        SmtpFailure::Endpoint(WholeFailure::Refused(_)) | SmtpFailure::NoStartTls => {
            Failure::Rejected
        }
        SmtpFailure::NoAuthMechanism => Failure::Unauthorized,
        SmtpFailure::Reply(reply) => by_code(reply),
        SmtpFailure::Unreadable => ChannelText::new(String::new())
            .unrecognized("an SMTP server answered with no reply SMTP knows"),
        SmtpFailure::Other(text) => text
            .unrecognized("an SMTP dialogue failed in a way its translation table does not know"),
        SmtpFailure::Endpoint(other) => ChannelText::new(format!("{other:?}")).unrecognized(
            "an SMTP server's endpoint failed in a way its translation table does not know",
        ),
    }
}

/// The failure a reply's code is (RFC 5321 section 4.2.3, RFC 4954
/// section 6): a transient reply is a server error, a refused login is
/// unauthorized, an unknown mailbox is gone and any other permanent reply
/// is a rejection.
fn by_code(reply: Reply) -> Failure {
    match (reply.code, reply.enhanced) {
        (421 | 450 | 451 | 452 | 454, _) => Failure::ServerError,
        (530 | 534 | 535 | 538, _) => Failure::Unauthorized,
        (550, [5, 1, 1 | 10]) | (551, _) => Failure::Gone,
        (500..=504 | 550 | 552..=555, _) => Failure::Rejected,
        (code, _) => {
            tracing::warn!(
                code,
                "an SMTP server answered with a reply its translation table does not know"
            );
            reply.text.unrecognized("an SMTP server's reply")
        }
    }
}
