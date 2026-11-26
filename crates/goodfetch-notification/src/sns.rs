//! The `sns` channel (ADR 0027; issue 3680): each Notification published
//! to an Amazon SNS topic, one `Publish` call per delivery through the
//! engine's Notification channel endpoint, signed with Signature Version 4
//! for the `sns` service in the channel's region. The message is the
//! Notification payload a `webhook` channel posts, in its `full` form and
//! its link without credentials, under the title as its subject. To a FIFO
//! topic the event ID goes as the deduplication ID, so a retry publishes
//! once, and the Run's ID as the message group. The channel is a
//! translation point (ADR 0127): its table maps the error code SNS answers
//! with, else the status, to the driver's [`Failure`].

use std::time::Duration;

use goodfetch_engine::aws_signing::{self, Credentials, Scope, Unsignable};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_protocol::notifications::NotificationData;
use hyper::body::Bytes;
use hyper::{HeaderMap, StatusCode};
use quick_xml::events::Event;
use url::Url;
use url::form_urlencoded;

use crate::api::{self, ApiFailure, ApiUnusable, Known, Table};
use crate::driver::Channel;
use crate::failure::Failure;
use crate::message::{Message, clipped};
use crate::webhook::{self, Payload};

/// How long a delivery waits for SNS's whole answer before it counts as
/// [`Failure::Timeout`].
pub const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// The service name SNS requests are signed for.
const SERVICE: &str = "sns";

/// The version of SNS's Query API the channel calls.
const API_VERSION: &str = "2010-03-31";

/// The longest subject SNS takes: under 100 characters.
const SUBJECT_LIMIT: usize = 99;

/// The message group of an event about no Run and no Schedule, on a FIFO
/// topic.
const NO_RUN_GROUP: &str = "goodfetch";

/// SNS's translation table.
const TABLE: Table = Table {
    party: "SNS",
    kind: error_code,
    wait: api::header_wait,
    known,
};

/// An `sns` channel: the topic it publishes to, and how it reaches SNS.
#[derive(Debug)]
pub struct Sns {
    name: String,
    topic_arn: String,
    fifo: bool,
    region: String,
    url: Url,
    credentials: Credentials,
    endpoint: NotificationEndpoint,
    clock: Clock,
}

/// Why SNS did not take the message.
#[derive(Debug)]
#[non_exhaustive]
pub enum SnsFailure {
    /// The access key cannot sign a request, so none was sent.
    Unsignable(Unsignable),
    /// No whole answer within [`SEND_TIMEOUT`].
    TimedOut,
    /// The endpoint's failure, or an answer that is no success, with the
    /// error code SNS gave, such as `AuthorizationError`.
    Api(ApiFailure),
}

impl Sns {
    /// The `sns` channel `name`, publishing to the topic `topic_arn` in
    /// `region`, at `endpoint_url` when given, else at SNS's regional
    /// endpoint, `https://sns.<region>.amazonaws.com/`, signed by
    /// `credentials` and reached through `endpoint`.
    ///
    /// # Errors
    ///
    /// [`ApiUnusable::Region`] or [`ApiUnusable::Topic`] when the region or
    /// the topic's ARN is unusable, [`ApiUnusable::Url`] for an endpoint URL
    /// that is no `http` or `https` URL with a host, and
    /// [`ApiUnusable::Refused`] when the egress policy refuses the endpoint
    /// (ADR 0017), so the configuration fails rather than each delivery.
    pub fn new(
        name: impl Into<String>,
        topic_arn: &str,
        region: &str,
        endpoint_url: Option<Url>,
        credentials: Credentials,
        endpoint: NotificationEndpoint,
        clock: Clock,
    ) -> Result<Self, ApiUnusable> {
        if !api::is_aws_region(region) {
            return Err(ApiUnusable::Region);
        }
        let fifo = topic_is_fifo(topic_arn).ok_or(ApiUnusable::Topic)?;
        let url = match endpoint_url {
            Some(url) => url,
            None => Url::parse(&format!("https://sns.{region}.amazonaws.com/"))
                .map_err(|_| ApiUnusable::Region)?,
        };
        api::usable_host(&url, &endpoint)?;
        Ok(Self {
            name: name.into(),
            topic_arn: topic_arn.to_owned(),
            fifo,
            region: region.to_owned(),
            url,
            credentials,
            endpoint,
            clock,
        })
    }

    /// The `Publish` call's form for `message`.
    fn form(&self, message: &Message) -> String {
        let mut full = webhook::payload(message, Payload::Full);
        if let NotificationData::Full(data) = &mut full.data {
            data.link = message
                .link_without_credentials()
                .map(|link| link.as_str().to_owned());
        }
        let payload = serde_json::to_string(&full).expect(
            "invariant: a Notification payload is strings, IDs, a Product and a point in time the engine's clock read, which always serialize",
        );
        let mut form = form_urlencoded::Serializer::new(String::new());
        form.append_pair("Action", "Publish")
            .append_pair("Version", API_VERSION)
            .append_pair("TopicArn", &self.topic_arn)
            .append_pair("Message", &payload);
        if let Some(subject) = subject(&message.title) {
            form.append_pair("Subject", &subject);
        }
        if self.fifo {
            let group = match (&message.run_id, &message.schedule_id) {
                (Some(run_id), _) => run_id.to_string(),
                (None, Some(schedule_id)) => schedule_id.as_str().to_owned(),
                (None, None) => NO_RUN_GROUP.to_owned(),
            };
            form.append_pair("MessageDeduplicationId", message.event_id.as_str())
                .append_pair("MessageGroupId", &group);
        }
        form.finish()
    }
}

impl Channel for Sns {
    type Failure = SnsFailure;

    fn name(&self) -> &str {
        &self.name
    }

    /// SNS's regional host: its publish quota is the account's in a region.
    fn destination(&self) -> &str {
        self.url.host_str().unwrap_or_default()
    }

    async fn send(&self, message: &Message) -> Result<(), SnsFailure> {
        let request = aws_signing::signed_post(
            &self.url,
            &[(
                "content-type",
                "application/x-www-form-urlencoded; charset=utf-8",
            )],
            Bytes::from(self.form(message)),
            Scope {
                region: &self.region,
                service: SERVICE,
            },
            &self.credentials,
            self.clock.wall(),
        )
        .map_err(SnsFailure::Unsignable)?;
        let sent = api::send(&self.endpoint, request, self.clock, &TABLE);
        tokio::select! {
            sent = sent => sent.map_err(SnsFailure::Api),
            () = self.clock.sleep(SEND_TIMEOUT) => Err(SnsFailure::TimedOut),
        }
    }

    fn map_failure(&self, failure: SnsFailure) -> Failure {
        match failure {
            SnsFailure::Unsignable(_) => Failure::Unauthorized,
            SnsFailure::TimedOut => Failure::Timeout,
            SnsFailure::Api(failure) => api::translate(failure, &TABLE),
        }
    }
}

/// Whether the topic `arn` names is a FIFO one; none when `arn` is no SNS
/// topic's: `arn:<partition>:sns:<region>:<account>:<topic>`, the topic
/// 1 to 256 letters, digits, `-` and `_`, a FIFO topic's with `.fifo`
/// after them.
fn topic_is_fifo(arn: &str) -> Option<bool> {
    let ["arn", partition, "sns", region, account, topic] = arn.split(':').collect::<Vec<_>>()[..]
    else {
        return None;
    };
    let name_usable = |name: &str| {
        (1..=256).contains(&name.len())
            && name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
    };
    let (name, fifo) = match topic.strip_suffix(".fifo") {
        Some(name) => (name, true),
        None => (topic, false),
    };
    let well_formed = !partition.is_empty()
        && api::is_aws_region(region)
        && account.len() == 12
        && account.chars().all(|character| character.is_ascii_digit())
        && name_usable(name);
    well_formed.then_some(fifo)
}

/// `title` as an SNS subject: each line break or other control character a
/// space, cut under 100 characters; none when nothing but spaces is left.
fn subject(title: &str) -> Option<String> {
    let flat: String = title
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let flat = flat.trim();
    (!flat.is_empty()).then(|| clipped(flat, SUBJECT_LIMIT))
}

/// The error code an answer names: its XML error's `Code`, such as
/// `AuthorizationError`, else its `x-amzn-ErrorType` header, before any
/// `:`.
fn error_code(headers: &HeaderMap, body: &[u8]) -> Option<String> {
    xml_code(body).or_else(|| {
        let named = headers.get("x-amzn-errortype")?.to_str().ok()?;
        Some(named.split(':').next().unwrap_or(named).trim().to_owned())
    })
}

/// The text of the first `Code` element of `body`; none when `body` is no
/// XML or has none.
fn xml_code(body: &[u8]) -> Option<String> {
    let mut reader = quick_xml::Reader::from_reader(body);
    let mut in_code = false;
    loop {
        match reader.read_event().ok()? {
            Event::Start(start) => in_code = start.local_name().as_ref() == b"Code",
            Event::Text(text) if in_code => {
                let code = text.decode().ok()?;
                return Some(code.trim().to_owned());
            }
            Event::End(_) => in_code = false,
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// The failure an SNS answer is, by its error code and else by its status,
/// if the table knows it. A `400` or `404` with a code the table does not
/// know stays unknown: SNS names every client error, so a new name is
/// noticed rather than guessed.
fn known(status: StatusCode, code: Option<&str>) -> Option<Known> {
    code.and_then(by_code).or_else(|| api::by_status(status))
}

/// The failure an error code SNS gives is, if the table knows it.
fn by_code(code: &str) -> Option<Known> {
    match code {
        "Throttled" | "Throttling" | "ThrottlingException" | "KMSThrottling" => {
            Some(Known::RateLimited)
        }
        "AuthorizationError"
        | "InvalidSecurity"
        | "KMSAccessDenied"
        | "KMSOptInRequired"
        | "AccessDenied"
        | "InvalidClientTokenId"
        | "SignatureDoesNotMatch"
        | "IncompleteSignature"
        | "MissingAuthenticationToken"
        | "ExpiredToken"
        | "RequestExpired" => Some(Known::Is(Failure::Unauthorized)),
        "NotFound" | "EndpointDisabled" => Some(Known::Is(Failure::Gone)),
        "InvalidParameter"
        | "ParameterValueInvalid"
        | "InvalidParameterValue"
        | "ValidationError"
        | "KMSDisabled"
        | "KMSInvalidState"
        | "KMSNotFound" => Some(Known::Is(Failure::Rejected)),
        "InternalError" | "InternalFailure" | "ServiceUnavailable" => {
            Some(Known::Is(Failure::ServerError))
        }
        _ => None,
    }
}
