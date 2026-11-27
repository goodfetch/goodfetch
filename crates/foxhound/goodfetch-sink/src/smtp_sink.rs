//! SMTP in-binary Sink (ADRs 0008, 0026, 0060 and 0127; issue 1163): delivers
//! committed Records as e-mail messages in Records mode (one message per
//! batch) over [`NotificationEndpoint::open_mail`], carrying
//! `X-Goodfetch-Delivery-Id` and `X-Goodfetch-Run-Epoch` headers so receivers
//! can deduplicate, proving [`GuaranteeClass::AtLeastOnce`].

use std::collections::BTreeMap;
use std::time::Duration;

use goodfetch_engine::http_client::WholeFailure;
use goodfetch_engine::notification_endpoint::{MailConnection, NotificationEndpoint};
use mail_builder::MessageBuilder;
use mail_builder::headers::text::Text;
use mail_send::SmtpClient;
use mail_send::smtp::message::Parameters;
use smtp_proto::{EXT_START_TLS, Response};
use url::Url;

use crate::contract::{
    Batch, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy, Written,
};

const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// Errors returned by [`SmtpSink`] before translation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SmtpError {
    /// Invalid SMTP destination URL or missing recipient/sender.
    #[error("invalid SMTP Sink configuration: {0}")]
    Config(String),
    /// Transport/confinement error opening or upgrading the mail connection.
    #[error("SMTP endpoint failure: {0:?}")]
    Endpoint(WholeFailure),
    /// Server does not advertise `STARTTLS` when TLS is required.
    #[error("SMTP server does not advertise STARTTLS")]
    NoStartTls,
    /// Unexpected SMTP protocol reply.
    #[error("SMTP server replied {code}: {message}")]
    Reply {
        /// Numeric SMTP reply code.
        code: u16,
        /// Server reply text.
        message: String,
    },
    /// Error from `mail-send`.
    #[error("{0}")]
    Client(#[from] mail_send::Error),
    /// Failed to serialize batch Records into message body.
    #[error("failed to serialize SMTP body: {0}")]
    Serialize(String),
    /// No active Tokio runtime available when called synchronously.
    #[error("SMTP Sink requires an active Tokio runtime")]
    NoRuntime,
}

/// Built-in SMTP Sink implementing [`Primitives`].
#[derive(Clone)]
pub struct SmtpSink {
    sink_name: String,
    host: String,
    port: u16,
    implicit_tls: bool,
    allow_insecure: bool,
    username: Option<String>,
    password: Option<String>,
    from: String,
    to: String,
    endpoint: NotificationEndpoint,
}

impl SmtpSink {
    /// Creates an [`SmtpSink`] from a destination URL:
    /// `smtp://[user:pass@]host[:port]?from=...&to=...[&allow_insecure=true]`
    /// or `smtps://...` (implicit TLS). `target_to` may supply the recipient
    /// `to` address if omitted from the URL query.
    ///
    /// # Errors
    ///
    /// [`SmtpError::Config`] if the URL, host, `from` or `to` is missing.
    pub fn from_url(
        sink_name: impl Into<String>,
        destination: &str,
        target_to: Option<&str>,
        endpoint: NotificationEndpoint,
    ) -> Result<Self, SmtpError> {
        let url = Url::parse(destination)
            .map_err(|err| SmtpError::Config(format!("invalid URL: {err}")))?;
        let implicit_tls = match url.scheme() {
            "smtps" => true,
            "smtp" => false,
            other => {
                return Err(SmtpError::Config(format!(
                    "unsupported scheme `{other}` (expected `smtp` or `smtps`)"
                )));
            }
        };
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| SmtpError::Config("missing SMTP host".into()))?
            .to_owned();
        let port = url.port().unwrap_or(if implicit_tls { 465 } else { 587 });
        let params: BTreeMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let allow_insecure = params
            .get("allow_insecure")
            .is_some_and(|v| v == "true" || v == "1");
        let from = params
            .get("from")
            .cloned()
            .unwrap_or_else(|| "goodfetch@localhost".to_owned());
        let to = params
            .get("to")
            .cloned()
            .or_else(|| target_to.map(str::to_owned))
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                SmtpError::Config("missing recipient (`to` query param or `target`)".into())
            })?;
        let username = (!url.username().is_empty()).then(|| url.username().to_owned());
        let password = url.password().map(str::to_owned);
        Ok(Self {
            sink_name: sink_name.into(),
            host,
            port,
            implicit_tls,
            allow_insecure,
            username,
            password,
            from,
            to,
            endpoint,
        })
    }

    async fn open_session(&self) -> Result<SmtpClient<MailConnection>, SmtpError> {
        let connection = self
            .endpoint
            .open_mail(&self.host, self.port, self.implicit_tls)
            .await
            .map_err(SmtpError::Endpoint)?;
        let local = connection.local_name();
        let mut client = SmtpClient {
            stream: connection,
            timeout: SEND_TIMEOUT,
        };
        expect_reply(client.read().await?, 220)?;
        let mut capabilities = client.ehlo(&local).await?;
        if !client.stream.is_secure() && !self.allow_insecure {
            if !capabilities.has_capability(EXT_START_TLS) {
                return Err(SmtpError::NoStartTls);
            }
            expect_reply(client.cmd(b"STARTTLS\r\n").await?, 220)?;
            let SmtpClient { stream, timeout } = client;
            let stream = stream.start_tls().await.map_err(SmtpError::Endpoint)?;
            client = SmtpClient { stream, timeout };
            capabilities = client.ehlo(&local).await?;
        }
        if let (Some(u), Some(p)) = (&self.username, &self.password) {
            let credentials = mail_send::Credentials::new(u.as_str(), p.as_str());
            client.authenticate(&credentials, &capabilities).await?;
        }
        Ok(client)
    }

    async fn send_batch(&self, batch: &Batch) -> Result<(), SmtpError> {
        if batch.records.is_empty() {
            return Ok(());
        }
        let delivery_id = if batch.records.len() == 1 {
            batch.records[0].delivery_id.as_str().to_owned()
        } else {
            let from = batch.records[0].number;
            let through = batch
                .records
                .last()
                .expect("invariant: batch non-empty")
                .number;
            format!(
                "{}/{}/{}/{from}-{through}",
                batch.run.run_id, self.sink_name, batch.dataset
            )
        };
        let mut body = String::new();
        for record in &batch.records {
            let line = serde_json::to_string(&record.value)
                .map_err(|err| SmtpError::Serialize(err.to_string()))?;
            body.push_str(&line);
            body.push('\n');
        }
        let subject = format!(
            "Goodfetch {} ({} records)",
            batch.dataset,
            batch.records.len()
        );
        let raw_message = MessageBuilder::new()
            .from(self.from.as_str())
            .to(self.to.as_str())
            .subject(subject)
            .header("X-Goodfetch-Delivery-Id", Text::new(delivery_id.as_str()))
            .header(
                "X-Goodfetch-Run-Epoch",
                Text::new(batch.run.run_epoch.to_string()),
            )
            .header("X-Goodfetch-Run-Id", Text::new(batch.run.run_id.as_str()))
            .text_body(body)
            .write_to_vec()
            .map_err(|err| SmtpError::Serialize(err.to_string()))?;

        let mut client = self.open_session().await?;
        let none = Parameters::new();
        client.mail_from(self.from.as_str(), &none).await?;
        client.rcpt_to(self.to.as_str(), &none).await?;
        client.data(&raw_message).await?;
        let _ = client.quit().await;
        Ok(())
    }
}

fn expect_reply(reply: Response<String>, code: u16) -> Result<(), SmtpError> {
    if reply.code == code {
        Ok(())
    } else {
        Err(SmtpError::Reply {
            code: reply.code,
            message: reply.message,
        })
    }
}

impl Primitives for SmtpSink {
    type Error = SmtpError;

    fn probe(&mut self, _run: &RunCopy) -> Result<Probe, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| SmtpError::NoRuntime)?;
        handle.block_on(async {
            let client = self.open_session().await?;
            let _ = client.quit().await;
            Ok::<_, SmtpError>(())
        })?;
        Ok(Probe {
            class: GuaranteeClass::AtLeastOnce,
            delivered: BTreeMap::new(),
        })
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| SmtpError::NoRuntime)?;
        handle.block_on(self.send_batch(batch))?;
        Ok(Written::Delivered)
    }

    fn map_failure(&self, error: Self::Error) -> Failure {
        map_smtp_error(error)
    }
}

/// Translates [`SmtpError`] into the driver's [`Failure`] (ADR 0127).
pub fn map_smtp_error(error: SmtpError) -> Failure {
    match error {
        SmtpError::Config(msg) | SmtpError::Serialize(msg) => {
            Failure::new(FailureKind::Permanent, msg)
        }
        SmtpError::NoStartTls => Failure::new(
            FailureKind::Permanent,
            "SMTP server does not advertise STARTTLS",
        ),
        SmtpError::NoRuntime => Failure::new(
            FailureKind::Permanent,
            "SMTP Sink requires an active Tokio runtime",
        ),
        SmtpError::Endpoint(err) => {
            let text = format!("{err:?}");
            match err {
                WholeFailure::Refused(_) => Failure::new(FailureKind::Permanent, text),
                WholeFailure::CertificateInvalid(_)
                | WholeFailure::ClientCertificate(_)
                | WholeFailure::Network => Failure::new(FailureKind::Transient, text),
                _ => {
                    tracing::warn!(error = %text, "unrecognized SMTP endpoint failure; treating as transient");
                    Failure::new(FailureKind::Unrecognized, text)
                }
            }
        }
        SmtpError::Reply { code, message } => {
            map_reply_code(code, format!("SMTP {code}: {message}"))
        }
        SmtpError::Client(err) => {
            let text = err.to_string();
            match err {
                mail_send::Error::AuthenticationFailed(_)
                | mail_send::Error::UnsupportedAuthMechanism => {
                    Failure::new(FailureKind::Auth, text)
                }
                mail_send::Error::Io(_) | mail_send::Error::Timeout => {
                    Failure::new(FailureKind::Transient, text)
                }
                mail_send::Error::UnexpectedReply(reply) => map_reply_code(reply.code, text),
                _ => {
                    tracing::warn!(error = %text, "unrecognized SMTP failure; treating as transient");
                    Failure::new(FailureKind::Unrecognized, text)
                }
            }
        }
    }
}

fn map_reply_code(code: u16, text: String) -> Failure {
    match code {
        400..=499 => Failure::new(FailureKind::Transient, text),
        530 | 535 => Failure::new(FailureKind::Auth, text),
        500..=599 => Failure::new(FailureKind::Permanent, text),
        _ => {
            tracing::warn!(error = %text, "unrecognized SMTP reply; treating as transient");
            Failure::new(FailureKind::Unrecognized, text)
        }
    }
}
