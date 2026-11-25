//! The shapes the token services answer in: `CapSolver`'s, 2Captcha's and
//! `CapMonster`'s create-task and task-result bodies, their task types and
//! error codes, as each documents them (docs.capsolver.com,
//! 2captcha.com/api-docs, capmonstercloud-docs.pages.dev).

use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Failed;

/// The user agent each service says it solved the widget with.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

/// A token service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Service {
    Capsolver,
    TwoCaptcha,
    Capmonster,
}

/// Every token service.
pub(super) const SERVICES: [Service; 3] =
    [Service::Capsolver, Service::TwoCaptcha, Service::Capmonster];

/// What a call is refused for, whichever service it came to.
#[derive(Clone, Copy)]
pub(super) enum Refusal {
    /// Its body is not one the service reads.
    Malformed,
    /// Its key is no account's.
    Key,
    /// Its task's type is none the service solves.
    Type,
    /// Its task names no page.
    PageUrl,
    /// Its task names no site key.
    SiteKey,
    /// It polls a task that is not the account's.
    Task,
}

impl Service {
    /// The host name the service answers on.
    pub(super) const fn host(self) -> &'static str {
        match self {
            Self::Capsolver => "capsolver.test",
            Self::TwoCaptcha => "two-captcha.test",
            Self::Capmonster => "capmonster.test",
        }
    }

    /// Whether the service solves the task type `kind`.
    pub(super) fn solves(self, kind: &str) -> bool {
        match self {
            Self::Capsolver => kind == "AntiTurnstileTaskProxyLess",
            Self::TwoCaptcha => matches!(kind, "TurnstileTaskProxyless" | "TurnstileTask"),
            Self::Capmonster => kind == "TurnstileTask",
        }
    }

    /// What a solved task costs, in dollars.
    pub(super) const fn price(self) -> &'static str {
        match self {
            Self::Capsolver => "0.0012",
            Self::TwoCaptcha => "0.00145",
            Self::Capmonster => "0.0013",
        }
    }

    /// The key of the account whose drawn half are `high` and `low`, in the
    /// service's format.
    pub(super) fn key(self, high: u64, low: u64) -> String {
        match self {
            Self::Capsolver => format!("CAP-{high:016X}{low:016X}"),
            Self::TwoCaptcha | Self::Capmonster => format!("{high:016x}{low:016x}"),
        }
    }

    /// The ID of the task numbered `number` whose drawn half are `high` and
    /// `low`: a UUID at `CapSolver`, an integer elsewhere, past 32 bits at
    /// 2Captcha, as its own are.
    pub(super) fn task_id(self, number: u64, high: u64, low: u64) -> Value {
        match self {
            Self::Capsolver => json!(super::uuid(high, low)),
            Self::TwoCaptcha => json!(70_000_000_000 + number),
            Self::Capmonster => json!(407_533_072 + number),
        }
    }

    /// The error of `refusal`.
    pub(super) fn refuse(self, refusal: Refusal) -> Failed {
        let code = match (refusal, self) {
            (Refusal::Malformed, Self::Capsolver) => "ERROR_BAD_REQUEST",
            (Refusal::Malformed | Refusal::SiteKey, Self::TwoCaptcha) => "ERROR_BAD_PARAMETERS",
            (Refusal::Malformed | Refusal::PageUrl | Refusal::SiteKey, Self::Capmonster) => {
                "ERROR_TASK_ABSENT"
            }
            (Refusal::Key, Self::Capsolver) => "ERROR_KEY_DENIED_ACCESS",
            (Refusal::Key, _) => "ERROR_KEY_DOES_NOT_EXIST",
            (Refusal::Type, _) => "ERROR_TASK_NOT_SUPPORTED",
            (Refusal::PageUrl | Refusal::SiteKey, Self::Capsolver) => "ERROR_INVALID_TASK_DATA",
            (Refusal::PageUrl, Self::TwoCaptcha) => "ERROR_PAGEURL",
            (Refusal::Task, Self::Capsolver) => "ERROR_TASKID_INVALID",
            (Refusal::Task, _) => "ERROR_NO_SUCH_CAPCHA_ID",
        };
        self.fail(code)
    }

    /// The error `code`: `CapSolver`'s carry a status of failure, the other
    /// services' come with `200`, and only 2Captcha numbers its errors.
    pub(super) fn fail(self, code: &str) -> Failed {
        let status = match (self, code) {
            (Self::Capsolver, "ERROR_KEY_DENIED_ACCESS") => StatusCode::UNAUTHORIZED,
            (Self::Capsolver, _) => StatusCode::BAD_REQUEST,
            (Self::TwoCaptcha | Self::Capmonster, _) => StatusCode::OK,
        };
        let id = match self {
            Self::TwoCaptcha => two_captcha_id(code),
            Self::Capsolver | Self::Capmonster => 1,
        };
        Failed {
            status,
            body: json!({
                "errorId": id,
                "errorCode": code,
                "errorDescription": description(code),
            }),
        }
    }

    /// The body that names the created task `id`.
    pub(super) fn created(self, id: &Value) -> Value {
        match self {
            Self::Capsolver => json!({ "errorId": 0, "status": "idle", "taskId": id }),
            Self::TwoCaptcha | Self::Capmonster => json!({ "errorId": 0, "taskId": id }),
        }
    }

    /// The body of a poll of the task `id`, which is not solved yet.
    pub(super) fn processing(self, id: &Value) -> Value {
        match self {
            Self::Capsolver => json!({
                "errorId": 0,
                "errorCode": null,
                "errorDescription": null,
                "status": "processing",
                "taskId": id,
            }),
            Self::TwoCaptcha => json!({ "errorId": 0, "status": "processing" }),
            Self::Capmonster => json!({
                "errorId": 0,
                "errorCode": null,
                "errorDescription": null,
                "status": "processing",
            }),
        }
    }

    /// The body of a poll of the task `id`, solved with `token`.
    pub(super) fn ready(self, id: &Value, token: &str) -> Value {
        match self {
            Self::Capsolver => json!({
                "errorId": 0,
                "errorCode": null,
                "errorDescription": null,
                "status": "ready",
                "taskId": id,
                "solution": { "token": token, "type": "turnstile", "userAgent": USER_AGENT },
            }),
            Self::TwoCaptcha => json!({
                "errorId": 0,
                "status": "ready",
                "solution": { "token": token, "userAgent": USER_AGENT },
                "cost": self.price(),
                "ip": "203.0.113.7",
                "createTime": 1_785_000_000,
                "endTime": 1_785_000_009,
                "solveCount": 1,
            }),
            Self::Capmonster => json!({
                "errorId": 0,
                "status": "ready",
                "solution": { "userAgent": USER_AGENT, "token": token },
            }),
        }
    }
}

/// 2Captcha's number for an error code, 1 for one it does not document.
fn two_captcha_id(code: &str) -> u64 {
    match code {
        "ERROR_NO_SLOT_AVAILABLE" => 2,
        "ERROR_PAGEURL" => 5,
        "ERROR_ZERO_BALANCE" => 10,
        "ERROR_IP_NOT_ALLOWED" => 11,
        "ERROR_CAPTCHA_UNSOLVABLE" => 12,
        "ERROR_NO_SUCH_METHOD" => 14,
        "ERROR_NO_SUCH_CAPCHA_ID" => 16,
        "ERROR_IP_BLOCKED" => 21,
        "ERROR_TASK_ABSENT" => 22,
        "ERROR_TASK_NOT_SUPPORTED" => 23,
        "ERROR_ACCOUNT_SUSPENDED" => 55,
        "ERROR_BAD_PARAMETERS" => 110,
        "ERROR_BAD_PROXY" => 130,
        _ => 1,
    }
}

/// What the error `code` says.
fn description(code: &str) -> &'static str {
    match code {
        "ERROR_ZERO_BALANCE" => "The account has no balance left.",
        "ERROR_KEY_DENIED_ACCESS" | "ERROR_KEY_DOES_NOT_EXIST" => {
            "The account's key is not accepted."
        }
        "ERROR_RATE_LIMIT" | "ERROR_TOO_MUCH_REQUESTS" => "Too many requests were sent.",
        "ERROR_CAPTCHA_UNSOLVABLE" => "The captcha could not be solved.",
        "ERROR_TASK_TIMEOUT" => "The captcha was not solved in time.",
        "ERROR_TASK_NOT_SUPPORTED" => "The type of captcha is not supported.",
        "ERROR_TASKID_INVALID" | "ERROR_NO_SUCH_CAPCHA_ID" => "The account has no such task.",
        "ERROR_PAGEURL" => "The task names no page.",
        "ERROR_BAD_REQUEST"
        | "ERROR_INVALID_TASK_DATA"
        | "ERROR_BAD_PARAMETERS"
        | "ERROR_TASK_ABSENT" => "The request misses what the task needs.",
        _ => "The service failed.",
    }
}
