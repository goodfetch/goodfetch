//! The solver-service stand-ins (#1539, ADRs 0035 and 0076): `CapSolver`,
//! 2Captcha and `CapMonster`, each on its own host name, answer their
//! create-task and poll calls in the service's own shapes with a token the
//! Proving ground's Turnstile page accepts, as the account of the call's key
//! was told, and record every call.
//!
//! Each test is a plain HTTP client of the service and of the page.
//! Covers: story:2-33

use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{Answer, ProvingGround, submission};

const SEED: u64 = 1539;

/// The task whose page holds the Turnstile widget.
const TASK: &str = "protection/3";

/// What differs between the token services.
#[derive(Clone, Copy)]
struct Service {
    /// Its name in `POST /solver-accounts`.
    name: &'static str,
    host: &'static str,
    /// Its task type for a Turnstile widget.
    kind: &'static str,
    /// A task type it does not support.
    unsupported: &'static str,
    /// What it answers that type.
    unsupported_error: fn() -> Value,
    /// The status it answers an error with.
    error_status: u16,
    /// What a solved task costs, in dollars.
    price: &'static str,
}

fn capsolver() -> Service {
    Service {
        name: "capsolver",
        host: "capsolver.test",
        kind: "AntiTurnstileTaskProxyLess",
        unsupported: "FunCaptchaTaskProxyLess",
        unsupported_error: || {
            json!({
                "errorId": 1,
                "errorCode": "ERROR_TASK_NOT_SUPPORTED",
                "errorDescription": "The type of captcha is not supported.",
            })
        },
        error_status: 400,
        price: "0.0012",
    }
}

fn two_captcha() -> Service {
    Service {
        name: "two_captcha",
        host: "two-captcha.test",
        kind: "TurnstileTaskProxyless",
        unsupported: "FunCaptchaTaskProxyless",
        unsupported_error: || {
            json!({
                "errorId": 23,
                "errorCode": "ERROR_TASK_NOT_SUPPORTED",
                "errorDescription": "The type of captcha is not supported.",
            })
        },
        error_status: 200,
        price: "0.00145",
    }
}

fn capmonster() -> Service {
    Service {
        name: "capmonster",
        host: "capmonster.test",
        kind: "TurnstileTask",
        unsupported: "FunCaptchaTask",
        unsupported_error: || {
            json!({
                "errorId": 1,
                "errorCode": "ERROR_TASK_NOT_SUPPORTED",
                "errorDescription": "The type of captcha is not supported.",
            })
        },
        error_status: 200,
        price: "0.0013",
    }
}

/// The text of `body` between `before` and the next `"`.
fn attribute(body: &str, before: &str) -> String {
    let from = body
        .find(before)
        .unwrap_or_else(|| panic!("{before} in {body}"))
        + before.len();
    let rest = &body[from..];
    rest[..rest.find('"').expect("the attribute ends")].to_owned()
}

/// The Turnstile page's URL and its widget's site key.
fn widget(proving_ground: &ProvingGround) -> (String, String) {
    let url = proving_ground.task(TASK)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned();
    let page = proving_ground.get(&url);
    assert!(
        page.body.contains("class=\"cf-turnstile\""),
        "{}",
        page.body
    );
    let site_key = attribute(&page.body, "data-sitekey=\"");
    (url, site_key)
}

/// One client of a service, on an account of its own.
struct Client<'a> {
    proving_ground: &'a ProvingGround,
    service: Service,
    account: String,
    key: String,
}

impl<'a> Client<'a> {
    /// A client whose account was told `told`.
    fn new(proving_ground: &'a ProvingGround, service: Service, told: &Value) -> Self {
        let answer = proving_ground.post_control(
            "/solver-accounts",
            &json!({ "service": service.name, "told": told }),
        );
        assert_eq!(answer.status, 200, "{}", answer.body);
        let account = answer.json();
        let text = |field: &str| account[field].as_str().expect(field).to_owned();
        Self {
            proving_ground,
            service,
            account: text("account"),
            key: text("key"),
        }
    }

    fn call(&self, path: &str, body: &Value) -> Answer {
        self.proving_ground
            .post_json(&self.proving_ground.url(self.service.host, path), body)
    }

    fn create(&self, kind: &str, url: &str, site_key: &str) -> Answer {
        self.call(
            "/createTask",
            &json!({
                "clientKey": self.key,
                "task": { "type": kind, "websiteURL": url, "websiteKey": site_key },
            }),
        )
    }

    fn create_task(&self, url: &str, site_key: &str) -> Value {
        let answer = self.create(self.service.kind, url, site_key);
        assert_eq!(answer.status, 200, "{}", answer.body);
        let created = answer.json();
        assert_eq!(created["errorId"], 0, "{created}");
        created["taskId"].clone()
    }

    fn poll(&self, task: &Value) -> Answer {
        self.call(
            "/getTaskResult",
            &json!({ "clientKey": self.key, "taskId": task }),
        )
    }

    /// The token of a task polled until it is ready.
    fn token(&self, url: &str, site_key: &str) -> String {
        let task = self.create_task(url, site_key);
        let waiting = self.poll(&task).json();
        assert_eq!(waiting["status"], "processing", "{waiting}");
        let ready = self.poll(&task).json();
        assert_eq!(ready["status"], "ready", "{ready}");
        ready["solution"]["token"]
            .as_str()
            .expect("a token")
            .to_owned()
    }

    /// The account's calls, as the stand-in recorded them.
    fn recorded(&self) -> Vec<Value> {
        let document = self.proving_ground.control("/solver-calls").json();
        document["calls"]
            .as_array()
            .expect("`calls` is an array")
            .iter()
            .filter(|call| call["account"] == self.account.as_str())
            .cloned()
            .collect()
    }
}

fn submit(proving_ground: &ProvingGround, url: &str, token: &str) -> Answer {
    proving_ground.post_form(url, &[("cf-turnstile-response", token)], None)
}

#[rstest]
#[case::capsolver(capsolver())]
#[case::two_captcha(two_captcha())]
#[case::capmonster(capmonster())]
fn the_page_accepts_the_token_a_service_answers_its_widgets_task_with(#[case] service: Service) {
    let proving_ground = ProvingGround::start(SEED);
    let (url, site_key) = widget(&proving_ground);
    let client = Client::new(&proving_ground, service, &json!("solve"));

    let accepted = submit(&proving_ground, &url, &client.token(&url, &site_key));
    let clearance = accepted.cookie("turnstile").expect("a clearance cookie");
    let page = proving_ground.get_with_cookie(&url, Some(&clearance));
    let record = json!({
        "id": attribute(&page.body, "data-id=\""),
        "title": attribute(&page.body.replace("</h1>", "\""), "<h1 class=\"title\">"),
    });
    let judged = proving_ground.judge(&submission(TASK, "success", &[record]));

    assert_eq!(accepted.status, 200, "{}", accepted.body);
    assert_eq!(judged["outcome"], "verified", "{judged}");
}

#[rstest]
#[case::capsolver(capsolver(), json!({ "errorId": 0, "status": "idle" }))]
#[case::two_captcha(two_captcha(), json!({ "errorId": 0 }))]
#[case::capmonster(capmonster(), json!({ "errorId": 0 }))]
fn a_service_names_the_created_task_in_its_own_shape(
    #[case] service: Service,
    #[case] shape: Value,
) {
    let proving_ground = ProvingGround::start(SEED);
    let (url, site_key) = widget(&proving_ground);
    let string = service.name == "capsolver";
    let client = Client::new(&proving_ground, service, &json!("solve"));

    let mut created = client.create(client.service.kind, &url, &site_key).json();
    let task = created
        .as_object_mut()
        .and_then(|fields| fields.remove("taskId"))
        .expect("a task ID");

    assert_eq!(created, shape);
    assert_eq!((task.is_string(), task.is_u64()), (string, !string));
}

#[test]
fn a_token_is_refused_unless_it_is_the_pages_own() {
    let proving_ground = ProvingGround::start(SEED);
    let (url, site_key) = widget(&proving_ground);
    let client = Client::new(&proving_ground, two_captcha(), &json!("solve"));
    let other_page = client.token(&proving_ground.url("elsewhere.test", "/"), &site_key);
    let other_widget = client.token(&url, "0x4AAAAAAAnotThePagesKey");

    let statuses = ["", "0.made-up.token", &other_page, &other_widget]
        .map(|token| submit(&proving_ground, &url, token).status);

    assert_eq!(statuses, [403; 4]);
}

#[test]
fn the_widgets_page_committed_as_a_record_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let record = json!({ "id": "T-0007", "title": "Just a moment..." });

    let judged = proving_ground.judge(&submission(TASK, "success", &[record]));

    assert_eq!(judged["outcome"], "false_success", "{judged}");
}

#[rstest]
#[case::capsolver(capsolver())]
#[case::two_captcha(two_captcha())]
#[case::capmonster(capmonster())]
fn a_kind_a_service_does_not_support_is_refused_with_no_price(#[case] service: Service) {
    let proving_ground = ProvingGround::start(SEED);
    let (url, site_key) = widget(&proving_ground);
    let client = Client::new(&proving_ground, service, &json!("solve"));
    let told = Client::new(&proving_ground, service, &json!("unsupported"));
    let expected = (client.service.unsupported_error)();

    let unsupported = client.create(client.service.unsupported, &url, &site_key);
    let told_unsupported = told.create(told.service.kind, &url, &site_key);

    for answer in [unsupported, told_unsupported] {
        assert_eq!(answer.status, client.service.error_status);
        assert_eq!(answer.json(), expected);
    }
    for (caller, kind) in [
        (&client, client.service.unsupported),
        (&told, told.service.kind),
    ] {
        assert_eq!(
            caller.recorded(),
            [json!({
                "sequence": caller.recorded()[0]["sequence"],
                "account": caller.account,
                "service": caller.service.name,
                "call": "create_task",
                "kind": kind,
                "task": null,
                "status": caller.service.error_status,
                "error": expected,
                "price": null,
            })]
        );
    }
}

#[rstest]
#[case::capsolver_balance(
    capsolver(),
    "ERROR_ZERO_BALANCE",
    400,
    json!({ "errorId": 1, "errorCode": "ERROR_ZERO_BALANCE", "errorDescription": "The account has no balance left." }),
)]
#[case::capsolver_key(
    capsolver(),
    "ERROR_KEY_DENIED_ACCESS",
    401,
    json!({ "errorId": 1, "errorCode": "ERROR_KEY_DENIED_ACCESS", "errorDescription": "The account's key is not accepted." }),
)]
#[case::two_captcha_balance(
    two_captcha(),
    "ERROR_ZERO_BALANCE",
    200,
    json!({ "errorId": 10, "errorCode": "ERROR_ZERO_BALANCE", "errorDescription": "The account has no balance left." }),
)]
#[case::capmonster_rate(
    capmonster(),
    "ERROR_TOO_MUCH_REQUESTS",
    200,
    json!({ "errorId": 1, "errorCode": "ERROR_TOO_MUCH_REQUESTS", "errorDescription": "Too many requests were sent." }),
)]
#[case::unknown_code(
    two_captcha(),
    "ERROR_NOBODY_KNOWS",
    200,
    json!({ "errorId": 1, "errorCode": "ERROR_NOBODY_KNOWS", "errorDescription": "The service failed." }),
)]
fn a_service_told_to_fail_a_create_answers_the_code_in_its_shape(
    #[case] service: Service,
    #[case] code: &str,
    #[case] status: u16,
    #[case] error: Value,
) {
    let proving_ground = ProvingGround::start(SEED);
    let (url, site_key) = widget(&proving_ground);
    let client = Client::new(&proving_ground, service, &json!({ "create_error": code }));

    let answer = client.create(client.service.kind, &url, &site_key);

    assert_eq!((answer.status, answer.json()), (status, error.clone()));
    let recorded = client.recorded();
    assert_eq!(
        (&recorded[0]["error"], &recorded[0]["price"]),
        (&error, &Value::Null)
    );
}

#[rstest]
#[case::capsolver(capsolver(), json!({ "errorId": 1, "errorCode": "ERROR_CAPTCHA_UNSOLVABLE", "errorDescription": "The captcha could not be solved." }))]
#[case::two_captcha(two_captcha(), json!({ "errorId": 12, "errorCode": "ERROR_CAPTCHA_UNSOLVABLE", "errorDescription": "The captcha could not be solved." }))]
#[case::capmonster(capmonster(), json!({ "errorId": 1, "errorCode": "ERROR_CAPTCHA_UNSOLVABLE", "errorDescription": "The captcha could not be solved." }))]
fn a_service_told_to_fail_a_task_answers_its_poll_with_the_error_and_charges_nothing(
    #[case] service: Service,
    #[case] error: Value,
) {
    let proving_ground = ProvingGround::start(SEED);
    let (url, site_key) = widget(&proving_ground);
    let client = Client::new(
        &proving_ground,
        service,
        &json!({ "result_error": "ERROR_CAPTCHA_UNSOLVABLE" }),
    );
    let task = client.create_task(&url, &site_key);

    let first = client.poll(&task);
    let second = client.poll(&task);

    assert_eq!(first.json()["status"], "processing", "{}", first.body);
    assert_eq!(
        (second.status, second.json()),
        (client.service.error_status, error.clone())
    );
    let errors: Vec<Value> = client
        .recorded()
        .iter()
        .map(|call| call["error"].clone())
        .collect();
    assert_eq!(errors, [Value::Null, Value::Null, error]);
    assert!(
        client.recorded().iter().all(|call| call["price"].is_null()),
        "{:?}",
        client.recorded()
    );
}

#[rstest]
#[case::capsolver(capsolver())]
#[case::two_captcha(two_captcha())]
#[case::capmonster(capmonster())]
fn a_service_told_to_stall_never_finishes_its_task(#[case] service: Service) {
    let proving_ground = ProvingGround::start(SEED);
    let (url, site_key) = widget(&proving_ground);
    let client = Client::new(&proving_ground, service, &json!("stall"));
    let task = client.create_task(&url, &site_key);

    let statuses: Vec<Value> = (0..5)
        .map(|_| client.poll(&task).json()["status"].clone())
        .collect();

    assert_eq!(statuses, ["processing"; 5]);
    assert!(client.recorded().iter().all(|call| call["price"].is_null()));
}

#[rstest]
#[case::capsolver(capsolver())]
#[case::two_captcha(two_captcha())]
#[case::capmonster(capmonster())]
fn a_service_told_to_answer_malformed_json_does_so_once_its_task_is_solved(
    #[case] service: Service,
) {
    let proving_ground = ProvingGround::start(SEED);
    let (url, site_key) = widget(&proving_ground);
    let client = Client::new(&proving_ground, service, &json!("malformed"));
    let task = client.create_task(&url, &site_key);

    client.poll(&task);
    let solved = client.poll(&task);

    assert_eq!(solved.status, 200);
    assert!(
        serde_json::from_str::<Value>(&solved.body).is_err(),
        "{}",
        solved.body
    );
}

#[rstest]
#[case::capsolver(capsolver(), ["ERROR_INVALID_TASK_DATA"; 2])]
#[case::two_captcha(two_captcha(), ["ERROR_PAGEURL", "ERROR_BAD_PARAMETERS"])]
#[case::capmonster(capmonster(), ["ERROR_TASK_ABSENT"; 2])]
fn a_task_with_no_page_or_no_site_key_is_refused(
    #[case] service: Service,
    #[case] codes: [&str; 2],
) {
    let proving_ground = ProvingGround::start(SEED);
    let (url, site_key) = widget(&proving_ground);
    let client = Client::new(&proving_ground, service, &json!("solve"));

    let refused = [("", site_key.as_str()), (url.as_str(), "")]
        .map(|(url, site_key)| client.create(service.kind, url, site_key).json());

    assert_eq!(
        refused.map(|error| error["errorCode"].clone()),
        codes.map(|code| json!(code))
    );
}

#[rstest]
#[case::capsolver(capsolver(), 401, "ERROR_KEY_DENIED_ACCESS", "ERROR_TASKID_INVALID")]
#[case::two_captcha(
    two_captcha(),
    200,
    "ERROR_KEY_DOES_NOT_EXIST",
    "ERROR_NO_SUCH_CAPCHA_ID"
)]
#[case::capmonster(
    capmonster(),
    200,
    "ERROR_KEY_DOES_NOT_EXIST",
    "ERROR_NO_SUCH_CAPCHA_ID"
)]
fn a_key_of_no_account_and_a_task_of_another_are_refused(
    #[case] service: Service,
    #[case] key_status: u16,
    #[case] key_code: &str,
    #[case] task_code: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let (url, site_key) = widget(&proving_ground);
    let client = Client::new(&proving_ground, service, &json!("solve"));
    let other = Client::new(&proving_ground, service, &json!("solve"));
    let task = client.create_task(&url, &site_key);

    let unknown = client.call(
        "/createTask",
        &json!({ "clientKey": "no-such-key", "task": { "type": client.service.kind } }),
    );
    let stolen = other.poll(&task);

    assert_eq!(
        (unknown.status, unknown.json()["errorCode"].clone()),
        (key_status, json!(key_code))
    );
    assert_eq!(stolen.json()["errorCode"], task_code, "{}", stolen.body);
}

#[rstest]
#[case::capsolver(capsolver())]
#[case::two_captcha(two_captcha())]
#[case::capmonster(capmonster())]
fn every_call_is_recorded_with_the_price_on_the_one_that_spends_it(#[case] service: Service) {
    let proving_ground = ProvingGround::start(SEED);
    let (url, site_key) = widget(&proving_ground);
    let client = Client::new(&proving_ground, service, &json!("solve"));

    client.token(&url, &site_key);
    let recorded = client.recorded();

    let task = &recorded[0]["task"];
    assert!(!task.is_null(), "{recorded:?}");
    let calls: Vec<Value> = recorded
        .iter()
        .map(|call| {
            json!([
                call["call"],
                call["kind"],
                call["task"],
                call["status"],
                call["error"],
                call["price"]
            ])
        })
        .collect();
    assert_eq!(
        calls,
        [
            json!([
                "create_task",
                client.service.kind,
                task,
                200,
                null,
                client.service.price
            ]),
            json!(["get_task_result", null, task, 200, null, null]),
            json!(["get_task_result", null, task, 200, null, null]),
        ]
    );
    let document = proving_ground.control("/solver-calls").body;
    assert!(!document.contains(&client.key), "{document}");
}

#[test]
fn an_account_of_no_service_or_told_something_else_is_refused() {
    let proving_ground = ProvingGround::start(SEED);

    let statuses = [
        json!({ "service": "no_such_service", "told": "solve" }),
        json!({ "service": "capsolver", "told": "sing" }),
        json!({ "service": "capsolver" }),
    ]
    .map(|account| {
        let answer = proving_ground.post_control("/solver-accounts", &account);
        (answer.status, answer.json()["error"].clone())
    });

    assert_eq!(
        statuses,
        [(); 3].map(|()| (400, json!("malformed_solver_account")))
    );
}
