//! The `email` channel (issue 1388): a Notification rendered as a plain-text
//! part and a minimal HTML part, its link without credentials, sent over
//! SMTP to a scripted local receiver, over TLS from the first byte or
//! upgraded with `STARTTLS` under a certificate of the test's own
//! authority, or posted to an `httpmock` stand-in of Amazon SES; and each
//! provider's translation table, one case per failure it knows and one it
//! does not. No real account is used.
#![expect(
    clippy::disallowed_types,
    reason = "the receivers stand in for a mail server's, outside the engine and any loom model: their own sockets, and a standard lock on what they received (ADR 0060)"
)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use data_encoding::BASE64;
use goodfetch_engine::aws_signing::Credentials;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::settings::{Layers, Policy, Resolved, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_notification::driver::Channel;
use goodfetch_notification::email::{Address, Email, Login, Provider, Ses, Smtp};
use goodfetch_notification::failure::Failure;
use goodfetch_notification::message::{EventId, Message};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::notifications::{NotificationEvent, SmtpSecurity};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::schedule::ScheduleId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::TestEnv;
use httpmock::MockServer;
use rstest::rstest;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use url::Url;

/// The name the SMTP receiver's certificate is for, mapped to loopback.
const HOST: &str = "mail.test";

/// When the reported event happened.
const OCCURRED_AT: u64 = 1_764_000_000;

/// The password in the Notification's link, which no email carries.
const LINK_PASSWORD: &str = "hunter2";

/// A runtime of real time: on a paused clock the channel's timeout would
/// fire before a receiver on the same thread answers (TESTING.md, Time).
fn runtime() -> tokio::runtime::Runtime {
    goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// A JSON table.
fn table(value: Value) -> serde_json::Map<String, Value> {
    let Value::Object(table) = value else {
        panic!("a layer is a table")
    };
    table
}

/// The machine's settings: the Operator policy's `policy`, and user
/// configuration's `configuration`.
fn settings(policy: Value, configuration: Value) -> Resolved {
    let layers = Layers {
        operator_policy: Policy {
            settings: Values::read(
                SettingsLayer::OperatorPolicy,
                &table(policy),
                Product::Collie,
            )
            .unwrap(),
            ..Policy::default()
        },
        user_configuration: Values::read(
            SettingsLayer::UserConfiguration,
            &table(configuration),
            Product::Collie,
        )
        .unwrap(),
        ..Layers::default()
    };
    resolve(&layers, Product::Collie, Product::Collie).unwrap()
}

/// The Notification each test sends, its link carrying a user and a
/// password.
fn message() -> Message {
    Message {
        event_id: EventId::new("evt_01JDQ8X2Y5").unwrap(),
        event: NotificationEvent::RUN_FAILED,
        run_id: Some(RunId::parse("nightly-prices").unwrap()),
        schedule_id: Some(ScheduleId::parse("prices").unwrap()),
        product: Product::Collie,
        title: "Run failed".to_owned(),
        text: "The Run nightly-prices failed: navigation_failed.".to_owned(),
        link: Some(
            Url::parse(&format!(
                "http://ops:{LINK_PASSWORD}@127.0.0.1:7777/runs/nightly-prices"
            ))
            .unwrap(),
        ),
        occurred_at: UNIX_EPOCH + Duration::from_secs(OCCURRED_AT),
    }
}

/// The endpoint `settings` let the channel reach, on `clock`.
fn endpoint(settings: &Resolved, clock: Clock) -> NotificationEndpoint {
    NotificationEndpoint::of(
        settings,
        clock,
        TaskOwner::root(BlockingPolicy::Inline).blocking_threads(),
    )
}

/// The channel sending from `goodfetch@example.com` to two addresses
/// through `provider`, on `clock`.
fn email(provider: Provider, clock: Clock) -> Email {
    Email::new(
        "ops",
        Address::parse("goodfetch@example.com").unwrap(),
        vec![
            Address::parse("ops@example.com").unwrap(),
            Address::parse("oncall@example.com").unwrap(),
        ],
        provider,
        clock,
    )
    .unwrap()
}

/// The login the SMTP channel gives.
fn login() -> Login {
    Login {
        username: SecretString::from("goodfetch".to_owned()),
        password: SecretString::from("smtp-password".to_owned()),
    }
}

/// The failure the driver gets: its name, whether it is retried, and the
/// wait it asks for.
fn summary(failure: &Failure) -> (&'static str, bool, Option<Duration>) {
    (
        failure.name(),
        failure.is_transient(),
        failure.retry_after(),
    )
}

/// How the receiver secures its connections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Security {
    /// TLS from the first byte.
    Tls,
    /// A plain connection that offers `STARTTLS`.
    StartTls,
    /// A plain connection that offers no `STARTTLS`.
    Plain,
}

/// A step of the dialogue whose reply a test scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Greeting,
    Auth,
    Mail,
    Rcpt,
    Data,
    EndOfData,
}

/// The receiver's replies: its scripted ones, else a success.
#[derive(Clone, Debug)]
struct Script {
    security: Security,
    replies: Vec<(Step, &'static str)>,
}

impl Script {
    fn reply(&self, step: Step) -> String {
        let scripted = self
            .replies
            .iter()
            .find(|(scripted, _)| *scripted == step)
            .map(|(_, reply)| *reply);
        let reply = scripted.unwrap_or(match step {
            Step::Greeting => "220 mail.test ESMTP ready",
            Step::Auth => "235 2.7.0 Authentication successful",
            Step::Mail => "250 2.1.0 Sender OK",
            Step::Rcpt => "250 2.1.5 Recipient OK",
            Step::Data => "354 Start mail input; end with <CRLF>.<CRLF>",
            Step::EndOfData => "250 2.0.0 Queued",
        });
        format!("{reply}\r\n")
    }

    /// The `EHLO` reply, offering `STARTTLS` on a plain connection that
    /// offers it.
    fn ehlo(&self, secured: bool) -> String {
        let start_tls = if self.security == Security::StartTls && !secured {
            "250-STARTTLS\r\n"
        } else {
            ""
        };
        format!("250-mail.test\r\n250-AUTH PLAIN\r\n{start_tls}250 8BITMIME\r\n")
    }
}

/// What the receiver got: each command line, and the message's lines.
#[derive(Clone, Debug, Default)]
struct Seen {
    connections: usize,
    commands: Vec<String>,
    data: String,
}

/// The receiver: its port, its authority's file, and what it got.
struct Receiver {
    port: u16,
    authority: std::path::PathBuf,
    seen: Arc<Mutex<Seen>>,
}

#[expect(
    clippy::disallowed_methods,
    reason = "the stand-in mail server, outside the engine, runs its accept loop on the test's runtime and writes its authority before any engine runs (ADR 0062)"
)]
fn receiver(runtime: &tokio::runtime::Runtime, env: &TestEnv, script: Script) -> Receiver {
    let authority_key = rcgen::KeyPair::generate().unwrap();
    let mut authority_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    authority_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let authority_certificate = authority_params.self_signed(&authority_key).unwrap();
    let authority = env.folder().join("authority.der");
    std::fs::write(&authority, authority_certificate.der()).unwrap();
    let issuer = rcgen::Issuer::new(authority_params, authority_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec![HOST.to_owned()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone()],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            0,
        )))
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let kept = Arc::clone(&seen);
    runtime.spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            kept.lock().unwrap().connections += 1;
            if script.security == Security::Tls {
                if let Ok(tls) = acceptor.accept(tcp).await {
                    converse(tls, &script, true, true, &kept).await;
                }
            } else if let Some(tcp) = converse(tcp, &script, false, true, &kept).await
                && let Ok(tls) = acceptor.accept(tcp).await
            {
                converse(tls, &script, true, false, &kept).await;
            }
        }
    });
    Receiver {
        port,
        authority,
        seen,
    }
}

/// One SMTP dialogue on `stream`, `secured` or not, greeting first when
/// `greet`: the stream back once the client asked for `STARTTLS`.
async fn converse<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    script: &Script,
    secured: bool,
    greet: bool,
    seen: &Mutex<Seen>,
) -> Option<S> {
    let mut stream = BufReader::new(stream);
    if greet {
        let greeting = script.reply(Step::Greeting);
        stream.write_all(greeting.as_bytes()).await.ok()?;
        if !greeting.starts_with('2') {
            return None;
        }
    }
    loop {
        let mut line = String::new();
        if stream.read_line(&mut line).await.ok()? == 0 {
            return None;
        }
        let command = line.trim_end().to_owned();
        let verb = command
            .split([' ', ':'])
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        seen.lock().unwrap().commands.push(command);
        let reply = match verb.as_str() {
            "EHLO" => script.ehlo(secured),
            "STARTTLS" => {
                stream
                    .write_all(b"220 2.0.0 Ready to start TLS\r\n")
                    .await
                    .ok()?;
                return Some(stream.into_inner());
            }
            "AUTH" => script.reply(Step::Auth),
            "MAIL" => script.reply(Step::Mail),
            "RCPT" => script.reply(Step::Rcpt),
            "DATA" => {
                let reply = script.reply(Step::Data);
                stream.write_all(reply.as_bytes()).await.ok()?;
                if !reply.starts_with("354") {
                    continue;
                }
                loop {
                    let mut line = String::new();
                    if stream.read_line(&mut line).await.ok()? == 0 {
                        return None;
                    }
                    if line == ".\r\n" {
                        break;
                    }
                    seen.lock().unwrap().data.push_str(&line);
                }
                script.reply(Step::EndOfData)
            }
            "QUIT" => {
                let _ = stream.write_all(b"221 2.0.0 Bye\r\n").await;
                return None;
            }
            _ => "500 5.5.2 Command unrecognized\r\n".to_owned(),
        };
        stream.write_all(reply.as_bytes()).await.ok()?;
    }
}

/// User configuration mapping [`HOST`] to loopback, trusting the
/// receiver's authority for it unless `untrusted`.
fn configuration(receiver: &Receiver, untrusted: bool) -> Value {
    let trusted_cas = if untrusted {
        json!({})
    } else {
        json!({ HOST: [receiver.authority.display().to_string()] })
    };
    json!({
        "network": { "resolve": { HOST: ["127.0.0.1"] } },
        "tls": { "trusted_cas": trusted_cas },
    })
}

/// The SMTP channel to the receiver, secured as `security`, logging in.
fn smtp(receiver: &Receiver, security: SmtpSecurity, settings: &Resolved, clock: Clock) -> Email {
    email(
        Provider::Smtp(Smtp::new(
            HOST,
            Some(receiver.port),
            security,
            Some(login()),
            endpoint(settings, clock),
        )),
        clock,
    )
}

/// What sending `message()` over SMTP to a receiver of `script`, secured
/// as `security`, came to, and what the receiver got.
fn sent_over_smtp(script: Script, security: SmtpSecurity) -> (Result<(), Failure>, Seen) {
    let env = TestEnv::new();
    let runtime = runtime();
    let receiver = receiver(&runtime, &env, script);
    let settings = settings(json!({}), configuration(&receiver, false));
    let channel = smtp(&receiver, security, &settings, Clock::system());
    let sent = runtime.block_on(channel.send(&message()));
    let seen = receiver.seen.lock().unwrap().clone();
    (sent.map_err(|failure| channel.map_failure(failure)), seen)
}

/// An email is sent over TLS from the first byte, or over a connection
/// upgraded with `STARTTLS` before the login: from the channel's address
/// to each of its own, as a plain-text part and an HTML part, its link
/// without the user and password it carried.
/// Covers: setting:notifications.channels
#[rstest]
#[case::implicit_tls(Security::Tls, SmtpSecurity::Tls)]
#[case::starttls(Security::StartTls, SmtpSecurity::Starttls)]
fn an_email_is_sent_with_both_parts_and_no_credential_in_its_link(
    #[case] offered: Security,
    #[case] security: SmtpSecurity,
) {
    let (sent, seen) = sent_over_smtp(
        Script {
            security: offered,
            replies: Vec::new(),
        },
        security,
    );

    let verbs = seen
        .commands
        .iter()
        .map(|command| {
            let mut words = command.split(' ');
            match words.next() {
                Some("AUTH") => format!("AUTH {}", words.next().unwrap_or_default()),
                Some("EHLO") => "EHLO".to_owned(),
                _ => command.clone(),
            }
        })
        .collect::<Vec<_>>();
    let expected_verbs = match offered {
        Security::StartTls => vec!["EHLO", "STARTTLS", "EHLO"],
        _ => vec!["EHLO"],
    }
    .into_iter()
    .chain([
        "AUTH PLAIN",
        "MAIL FROM:<goodfetch@example.com>",
        "RCPT TO:<ops@example.com>",
        "RCPT TO:<oncall@example.com>",
        "DATA",
        "QUIT",
    ])
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let data = seen.data.to_ascii_lowercase();
    assert_eq!(
        (
            sent,
            verbs,
            data.contains("content-type: multipart/alternative"),
            data.contains("content-type: text/plain"),
            data.contains("content-type: text/html"),
            seen.data
                .matches("http://127.0.0.1:7777/runs/nightly-prices")
                .count(),
            seen.data.contains(LINK_PASSWORD),
            data.contains("message-id: <evt_01jdq8x2y5@example.com>"),
        ),
        (Ok(()), expected_verbs, true, true, true, 3, false, true)
    );
}

/// The SMTP translation table: each reply the driver knows, at the step
/// that gave it, maps to its failure, retried or not; a reply it does not
/// know, or none it can read, is `unrecognized` and retried.
/// Covers: setting:notifications.channels
#[rstest]
#[case::greeting_unavailable(Security::StartTls, Step::Greeting, "421 4.3.2 Service not available", ("server_error", true))]
#[case::greeting_refused(Security::StartTls, Step::Greeting, "554 5.7.1 No SMTP service here", ("rejected", false))]
#[case::credentials_refused(Security::StartTls, Step::Auth, "535 5.7.8 Authentication credentials invalid", ("unauthorized", false))]
#[case::mechanism_too_weak(Security::StartTls, Step::Auth, "534 5.7.9 Authentication mechanism is too weak", ("unauthorized", false))]
#[case::authentication_unavailable(Security::StartTls, Step::Auth, "454 4.7.0 Temporary authentication failure", ("server_error", true))]
#[case::authentication_required(Security::StartTls, Step::Mail, "530 5.7.0 Authentication required", ("unauthorized", false))]
#[case::sender_refused(Security::StartTls, Step::Mail, "553 5.1.8 Sender address rejected", ("rejected", false))]
#[case::command_refused(Security::StartTls, Step::Mail, "502 5.5.1 Command not implemented", ("rejected", false))]
#[case::storage_short(Security::StartTls, Step::Mail, "452 4.3.1 Insufficient system storage", ("server_error", true))]
#[case::mailbox_unknown(Security::StartTls, Step::Rcpt, "550 5.1.1 User unknown", ("gone", false))]
#[case::user_not_local(Security::StartTls, Step::Rcpt, "551 5.1.6 User not local", ("gone", false))]
#[case::mailbox_refused(Security::StartTls, Step::Rcpt, "550 5.7.1 Relaying denied", ("rejected", false))]
#[case::mailbox_busy(Security::StartTls, Step::Rcpt, "450 4.2.1 Mailbox busy", ("server_error", true))]
#[case::data_refused(Security::StartTls, Step::Data, "554 5.5.1 No valid recipients", ("rejected", false))]
#[case::message_too_big(Security::StartTls, Step::EndOfData, "552 5.3.4 Message too big", ("rejected", false))]
#[case::processing_error(Security::StartTls, Step::EndOfData, "451 4.3.0 Local error in processing", ("server_error", true))]
#[case::unknown_reply(Security::StartTls, Step::EndOfData, "599 9.9.9 Something new", ("unrecognized", true))]
#[case::unreadable_greeting(Security::StartTls, Step::Greeting, "hello", ("unrecognized", true))]
#[case::no_starttls(Security::Plain, Step::Greeting, "220 mail.test ESMTP ready", ("rejected", false))]
fn each_smtp_reply_maps_to_its_failure(
    #[case] offered: Security,
    #[case] step: Step,
    #[case] reply: &'static str,
    #[case] expected: (&str, bool),
) {
    let (sent, _) = sent_over_smtp(
        Script {
            security: offered,
            replies: vec![(step, reply)],
        },
        SmtpSecurity::Starttls,
    );

    let failure = sent.unwrap_err();
    assert_eq!(
        (failure.name(), failure.is_transient()),
        (expected.0, expected.1)
    );
}

/// A server nobody listens on, or one whose certificate is not trusted,
/// is `unreachable` and retried; one on a private address under
/// `egress.no_private_endpoints` is refused before any connection,
/// `rejected` and not retried.
/// Covers: setting:notifications.channels, setting:egress.no_private_endpoints
#[rstest]
#[case::closed_port("closed", ("unreachable", true, None), 0)]
#[case::untrusted_certificate("untrusted", ("unreachable", true, None), 1)]
#[case::private_server("private", ("rejected", false, None), 0)]
fn an_unreached_server_maps_to_its_failure(
    #[case] reach: &str,
    #[case] expected: (&str, bool, Option<Duration>),
    #[case] connections: usize,
) {
    let env = TestEnv::new();
    let runtime = runtime();
    let receiver = receiver(
        &runtime,
        &env,
        Script {
            security: Security::Tls,
            replies: Vec::new(),
        },
    );
    let policy = if reach == "private" {
        json!({ "egress": { "no_private_endpoints": true } })
    } else {
        json!({})
    };
    let settings = settings(policy, configuration(&receiver, reach == "untrusted"));
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = if reach == "closed" {
        closed.local_addr().unwrap().port()
    } else {
        receiver.port
    };
    drop(closed);
    let channel = email(
        Provider::Smtp(Smtp::new(
            HOST,
            Some(port),
            SmtpSecurity::Tls,
            Some(login()),
            endpoint(&settings, Clock::system()),
        )),
        Clock::system(),
    );

    let failure = runtime.block_on(channel.send(&message())).unwrap_err();

    assert_eq!(
        (
            summary(&channel.map_failure(failure)),
            receiver.seen.lock().unwrap().connections
        ),
        (expected, connections)
    );
}

/// A server that takes the connection and never greets is a `timeout`,
/// retried, once the channel's wait has passed on the paused clock.
/// Covers: setting:notifications.channels
#[test]
fn a_server_that_never_greets_times_out() {
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = silent.local_addr().unwrap().port();

    let failure = goodfetch_test_support::runtime().block_on(async {
        #[expect(
            clippy::disallowed_methods,
            reason = "the channel's wait runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
        )]
        let clock = Clock::paused(UNIX_EPOCH + Duration::from_secs(OCCURRED_AT));
        let channel = email(
            Provider::Smtp(Smtp::new(
                "127.0.0.1",
                Some(port),
                SmtpSecurity::Starttls,
                None,
                endpoint(&settings(json!({}), json!({})), clock),
            )),
            clock,
        );
        let failure = channel.send(&message()).await.unwrap_err();
        summary(&channel.map_failure(failure))
    });

    drop(silent);
    assert_eq!(failure, ("timeout", true, None));
}

/// The access key SES requests are signed with.
fn credentials(access_key_id: &str) -> Credentials {
    Credentials {
        access_key_id: access_key_id.to_owned(),
        secret_access_key: SecretString::from(
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned(),
        ),
        session_token: None,
    }
}

/// What SES received.
#[derive(Clone, Debug)]
struct Posted {
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

/// An `httpmock` stand-in of SES answering `status`, with `headers` and
/// `body`, keeping every request it got.
fn ses_stand_in(
    status: u16,
    headers: &[(&str, &str)],
    body: &str,
) -> (MockServer, Arc<Mutex<Vec<Posted>>>) {
    let server = MockServer::start();
    let posted = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&posted);
    server.mock(|when, then| {
        when.is_true(move |request| {
            let authorization = request
                .headers_vec()
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.clone());
            kept.lock().unwrap().push(Posted {
                path: request.uri().path().to_owned(),
                authorization,
                body: request.body_vec(),
            });
            true
        });
        let then = headers
            .iter()
            .fold(then.status(status), |then, (name, value)| {
                then.header(*name, *value)
            });
        then.body(body);
    });
    (server, posted)
}

/// The SES channel posting to `server` in `eu-west-1`, signed by the
/// access key `access_key_id`.
fn ses(server: &MockServer, access_key_id: &str, clock: Clock) -> Email {
    email(
        Provider::Ses(
            Ses::new(
                "eu-west-1",
                Some(Url::parse(&server.base_url()).unwrap()),
                credentials(access_key_id),
                endpoint(&settings(json!({}), json!({})), clock),
            )
            .unwrap(),
        ),
        clock,
    )
}

/// An email through SES is one `SendEmail` call signed for `ses` in the
/// channel's region, its raw message holding both parts and its link
/// without credentials.
/// Covers: setting:notifications.channels
#[test]
fn an_email_through_ses_is_a_signed_raw_message() {
    let (server, posted) = ses_stand_in(200, &[], r#"{"MessageId":"0102-abc"}"#);
    let runtime = runtime();
    let channel = ses(&server, "AKIDEXAMPLE", Clock::system());

    let sent = runtime.block_on(channel.send(&message()));

    let posted = posted.lock().unwrap().clone();
    let [request] = posted.as_slice() else {
        panic!("one request, got {posted:?}")
    };
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    let raw = BASE64
        .decode(
            body["Content"]["Raw"]["Data"]
                .as_str()
                .unwrap_or_default()
                .as_bytes(),
        )
        .unwrap();
    let raw = String::from_utf8(raw).unwrap().to_ascii_lowercase();
    let authorization = request.authorization.clone().unwrap_or_default();
    assert_eq!(
        (
            sent.map_err(|failure| channel.map_failure(failure)),
            request.path.as_str(),
            authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/"),
            authorization.contains("/eu-west-1/ses/aws4_request"),
            body["FromEmailAddress"].clone(),
            body["Destination"]["ToAddresses"].clone(),
            raw.contains("content-type: text/plain"),
            raw.contains("content-type: text/html"),
            raw.contains(LINK_PASSWORD),
        ),
        (
            Ok(()),
            "/v2/email/outbound-emails",
            true,
            true,
            json!("goodfetch@example.com"),
            json!(["ops@example.com", "oncall@example.com"]),
            true,
            true,
            false,
        )
    );
}

/// The SES translation table: each answer the driver knows, by the error
/// type SES names in its header or body, or by its status, maps to its
/// failure, retried or not, with the wait a `Retry-After` asks for; an
/// answer it does not know is `unrecognized` and retried.
/// Covers: setting:notifications.channels
#[rstest]
#[case::too_many_requests(429, &[("x-amzn-ErrorType", "TooManyRequestsException:http://internal.amazon.com/coral/com.amazonaws.sesv2/"), ("retry-after", "60")], "{}", ("rate_limited", true, Some(Duration::from_mins(1))))]
#[case::quota_exceeded(400, &[("x-amzn-ErrorType", "LimitExceededException")], "{}", ("rate_limited", true, None))]
#[case::throttled_in_body(400, &[], r#"{"__type":"com.amazonaws.sesv2#ThrottlingException"}"#, ("rate_limited", true, None))]
#[case::access_denied(403, &[("x-amzn-ErrorType", "AccessDeniedException")], "{}", ("unauthorized", false, None))]
#[case::unknown_access_key(403, &[("x-amzn-ErrorType", "UnrecognizedClientException")], "{}", ("unauthorized", false, None))]
#[case::bad_signature(403, &[("x-amzn-ErrorType", "InvalidSignatureException")], "{}", ("unauthorized", false, None))]
#[case::expired_token(403, &[("x-amzn-ErrorType", "ExpiredTokenException")], "{}", ("unauthorized", false, None))]
#[case::account_suspended(400, &[("x-amzn-ErrorType", "AccountSuspendedException")], "{}", ("unauthorized", false, None))]
#[case::sending_paused(400, &[("x-amzn-ErrorType", "SendingPausedException")], "{}", ("unauthorized", false, None))]
#[case::message_rejected(400, &[("x-amzn-ErrorType", "MessageRejected")], "{}", ("rejected", false, None))]
#[case::sender_unverified(400, &[("x-amzn-ErrorType", "MailFromDomainNotVerifiedException")], "{}", ("rejected", false, None))]
#[case::bad_request(400, &[("x-amzn-ErrorType", "BadRequestException")], "{}", ("rejected", false, None))]
#[case::not_found(404, &[("x-amzn-ErrorType", "NotFoundException")], "{}", ("rejected", false, None))]
#[case::internal_failure(500, &[("x-amzn-ErrorType", "InternalFailure")], "{}", ("server_error", true, None))]
#[case::service_unavailable(503, &[], "", ("server_error", true, None))]
#[case::unknown_type(400, &[("x-amzn-ErrorType", "SomethingNewException")], "{}", ("unrecognized", true, None))]
#[case::unknown_status(418, &[], "", ("unrecognized", true, None))]
fn each_ses_answer_maps_to_its_failure(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] body: &str,
    #[case] expected: (&str, bool, Option<Duration>),
) {
    let (server, _) = ses_stand_in(status, headers, body);
    let runtime = runtime();
    let channel = ses(&server, "AKIDEXAMPLE", Clock::system());

    let failure = runtime.block_on(channel.send(&message())).unwrap_err();

    assert_eq!(summary(&channel.map_failure(failure)), expected);
}

/// An access key no HTTP header may carry is never sent: `unauthorized`,
/// not retried.
/// Covers: setting:notifications.channels
#[test]
fn an_unsignable_access_key_is_unauthorized() {
    let (server, posted) = ses_stand_in(200, &[], "{}");
    let runtime = runtime();
    let channel = ses(&server, "AKID\nEXAMPLE", Clock::system());

    let failure = runtime.block_on(channel.send(&message())).unwrap_err();

    assert_eq!(
        (
            summary(&channel.map_failure(failure)),
            posted.lock().unwrap().len()
        ),
        (("unauthorized", false, None), 0)
    );
}
