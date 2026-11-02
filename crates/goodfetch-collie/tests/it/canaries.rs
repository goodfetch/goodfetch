//! Secret canaries searched in every output of Collie's executable (spec
//! #564, ticket #649; ADRs 0067 and 0092): each kind of secret Collie holds
//! at M1 is planted where Collie takes it, a credential read through each
//! built-in Secret provider, an endpoint token and a Session cookie a
//! login sets, and searched in every reply and event, the logs, the Run
//! trace, run state, the Run index, Checkpoints, the Audit log, every other
//! file the test's folder holds and the OTLP export an in-process collector
//! receives, raw and in Goodfetch's encodings.
//!
//! [`planting`] is the registry of every kind test support holds a canary
//! for: where each is planted, or the ticket that plants it once Collie
//! takes it. A kind registered as planted that no test of this crate plants
//! fails [`every_kind_registered_as_planted_is_planted_by_its_test`].
//!
//! The positive control, the search finding what a `Mask` letting values
//! through leaves, and the short-value rule run in process, in
//! `goodfetch-engine`'s `engine::tests::masking`; the redaction property
//! tests in its `mask::tests` and in this crate's log formats.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the stand-ins' files and run the providers' stand-in server on a thread of its own"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the Secret providers' stand-in server listens outside the engine (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

use goodfetch_test_support::{
    Canaries, Output, ProvingGround, SecretKind, TestEnv, browser_cache, chrome_for_testing_files,
};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use prost::Message as _;
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::page::{opened, started};
use crate::run::call;

/// Where a kind of secret is planted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Planting {
    /// By the test `test` of this crate's file `file`.
    By {
        file: &'static str,
        test: &'static str,
    },
    /// Not yet: Collie takes no secret of the kind before the ticket that
    /// plants it.
    Pending { ticket: u32 },
    /// Only by the other Product, whose tests plant it.
    Foxhound,
}

/// The registry: where each kind of secret test support holds a canary
/// for is planted through Collie. Each later ticket that makes Collie take
/// a kind moves it from `Pending` to `By`.
fn planting(kind: SecretKind) -> Planting {
    match kind {
        SecretKind::RunDataKey | SecretKind::EndpointToken => Planting::By {
            file: "canaries.rs",
            test: "a_key_read_through_each_provider_and_an_endpoint_token_are_in_no_output",
        },
        SecretKind::SessionCookie => Planting::By {
            file: "canaries.rs",
            test: "a_session_cookie_a_login_sets_is_in_no_output",
        },
        SecretKind::ProxyPassword => Planting::By {
            file: "proxy.rs",
            test: "the_credentials_of_a_socks5_proxy_reach_it_through_the_sessions_loopback_relay_alone",
        },
        SecretKind::TypedPassword => Planting::Pending { ticket: 655 },
        SecretKind::TotpSeed => Planting::Pending { ticket: 658 },
        SecretKind::OneTimeCode => Planting::Pending { ticket: 734 },
        SecretKind::HttpAuthenticationPassword => Planting::Pending { ticket: 657 },
        SecretKind::HeaderValue => Planting::Pending { ticket: 656 },
        SecretKind::CallerAnswer => Planting::Pending { ticket: 652 },
        SecretKind::ModelApiKey => Planting::Pending { ticket: 934 },
        SecretKind::WebhookSigningSecret => Planting::Pending { ticket: 666 },
        SecretKind::SinkCredential => Planting::Foxhound,
    }
}

/// The source of each file the registry names.
const SOURCES: [(&str, &str); 2] = [
    ("canaries.rs", include_str!("canaries.rs")),
    ("proxy.rs", include_str!("proxy.rs")),
];

/// The body of the test function `test` in `source`: from its `fn` line
/// to the next item at the file's top level.
fn body_of<'s>(source: &'s str, test: &str) -> Option<&'s str> {
    let start = source.find(&format!("\nfn {test}("))?;
    let rest = &source[start + 1..];
    let end = rest[1..]
        .find("\n}\n")
        .map_or(rest.len(), |end| end + 1 + "\n}\n".len());
    Some(&rest[..end])
}

/// Every kind registered as planted names a test of this crate whose body
/// plants a canary of that kind; a kind the registry calls planted but no
/// test plants fails here. Every other kind is pending a named ticket, or
/// is Foxhound's alone.
///
/// Covers: story:4-91
#[test]
fn every_kind_registered_as_planted_is_planted_by_its_test() {
    let unplanted: Vec<String> = SecretKind::ALL
        .into_iter()
        .filter_map(|kind| match planting(kind) {
            Planting::By { file, test } => {
                let source = SOURCES
                    .iter()
                    .find(|(name, _)| *name == file)
                    .map(|(_, source)| *source)?;
                let plants = body_of(source, test)
                    .is_some_and(|body| body.contains(&format!("SecretKind::{kind:?}")));
                (!plants).then(|| format!("{kind:?} by {file}'s {test}"))
            }
            Planting::Pending { ticket } => (ticket == 0).then(|| format!("{kind:?}")),
            Planting::Foxhound => None,
        })
        .collect();
    let named: Vec<&str> = SecretKind::ALL
        .into_iter()
        .filter_map(|kind| match planting(kind) {
            Planting::By { file, .. } => Some(file),
            _ => None,
        })
        .filter(|file| SOURCES.iter().all(|(name, _)| name != file))
        .collect();
    assert_eq!(
        (unplanted, named),
        (Vec::<String>::new(), Vec::<&str>::new())
    );
}

/// A built-in Secret provider a Run's key-encryption key is read through.
#[derive(Clone, Copy, Debug)]
enum Provider {
    Env,
    SecretFile,
    /// The vendor CLIs' stand-ins are shell scripts, so these two run on Unix.
    #[cfg(unix)]
    OnePassword,
    #[cfg(unix)]
    Bitwarden,
    Vault,
    AwsSecretsManager,
    GcpSecretManager,
    AzureKeyVault,
}

/// What a test sets for a provider: its user configuration lines and the
/// engine's environment.
struct Configured {
    configuration: String,
    variables: Vec<(String, String)>,
}

impl Provider {
    /// Plants `key`, canary `id` of `canaries`, where the provider reads
    /// it, the stand-in server at `server` answering the HTTP ones.
    fn configure(
        self,
        env: &TestEnv,
        canaries: &mut Canaries,
        id: &str,
        key: &str,
        server: &str,
    ) -> Configured {
        let (reference, settings, variables): (&str, String, Vec<(&str, String)>) = match self {
            Self::Env => (
                "env:GF_TEST_RUN_KEY",
                String::new(),
                vec![("GF_TEST_RUN_KEY", key.to_owned())],
            ),
            Self::SecretFile => {
                let identity = crate::secrets::identity(env);
                crate::secrets::encrypt(env, "run-key", key, &identity);
                ("secretfile:run-key", String::new(), Vec::new())
            }
            #[cfg(unix)]
            Self::OnePassword | Self::Bitwarden => {
                return vendor(self, env, canaries, id, key);
            }
            Self::Vault => (
                "vault:secret/shop/portal#password",
                format!("secret_providers.vault.address = \"{server}\"\n"),
                vec![("VAULT_TOKEN", "stand-in-vault-token".to_owned())],
            ),
            Self::AwsSecretsManager => (
                "aws-sm:prod/portal#password",
                format!(
                    "secret_providers.aws_sm.region = \"eu-west-3\"\nsecret_providers.aws_sm.endpoint = \"{server}\"\n"
                ),
                vec![
                    ("AWS_ACCESS_KEY_ID", "stand-in-access-key".to_owned()),
                    ("AWS_SECRET_ACCESS_KEY", "stand-in-secret-access".to_owned()),
                ],
            ),
            Self::GcpSecretManager => (
                "gcp-sm:shop/portal/latest",
                format!("secret_providers.gcp_sm.endpoint = \"{server}\"\n"),
                vec![("GOOGLE_OAUTH_ACCESS_TOKEN", "stand-in-gcp-token".to_owned())],
            ),
            Self::AzureKeyVault => (
                "azure-kv:shop-vault/portal",
                format!(
                    "secret_providers.azure_kv.endpoint = \"{server}/{{vault}}\"\nsecret_providers.azure_kv.authority = \"{server}\"\n"
                ),
                vec![
                    ("AZURE_TENANT_ID", "shop-tenant".to_owned()),
                    ("AZURE_CLIENT_ID", "stand-in-client".to_owned()),
                    ("AZURE_CLIENT_SECRET", "stand-in-client-secret".to_owned()),
                ],
            ),
        };
        Configured {
            configuration: format!("run_state.key_reference = \"{reference}\"\n{settings}"),
            variables: variables
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        }
    }
}

/// The vendor CLI's stand-in for 1Password's `op` or Bitwarden's `bw`,
/// answering the key from a file of the test's folder, where it is planted.
#[cfg(unix)]
fn vendor(
    provider: Provider,
    env: &TestEnv,
    canaries: &mut Canaries,
    id: &str,
    key: &str,
) -> Configured {
    use std::os::unix::fs::PermissionsExt as _;

    use crate::secrets::external_providers::{SIGN_IN, Vendor};

    let vendor = match provider {
        Provider::OnePassword => Vendor::OnePassword,
        _ => Vendor::Bitwarden,
    };
    let folder = env.folder().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let store = folder.join("value");
    std::fs::write(&store, key).unwrap();
    canaries.record_planted(id, &store.display().to_string());
    let command = folder.join(vendor.scheme());
    std::fs::write(&command, vendor.script(&store)).unwrap();
    std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
    Configured {
        configuration: format!(
            "run_state.key_reference = \"{}\"\nsecret_providers.{}.command = \"{}\"\n",
            vendor.reference(),
            vendor.scheme(),
            command.display()
        ),
        variables: vec![(vendor.sign_in().to_owned(), SIGN_IN.to_owned())],
    }
}

/// The stand-in server of the HTTP providers on loopback, on a thread of
/// its own, each answering `value` as its documented API does: Vault's KV
/// v2 read, AWS Secrets Manager's `GetSecretValue`, GCP Secret Manager's
/// `AccessSecretVersion`, and Microsoft Entra ID's token and Azure Key
/// Vault's Get Secret. Answers its address.
fn stand_in(value: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            serve(stream, &value);
        }
    });
    address
}

/// Serves one Connection of the stand-in, request by request.
fn serve(stream: TcpStream, value: &str) {
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    loop {
        let mut target = String::new();
        let mut headers = BTreeMap::new();
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            if line == "\r\n" {
                break;
            }
            if target.is_empty() {
                line.split(' ')
                    .nth(1)
                    .unwrap_or_default()
                    .clone_into(&mut target);
            } else if let Some((name, field)) = line.split_once(':') {
                headers.insert(name.to_ascii_lowercase(), field.trim().to_owned());
            }
        }
        let length = headers
            .get("content-length")
            .map_or(0, |length| length.parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let (status, answer) = answer(&target, &headers, value);
        let answer = answer.to_string();
        let _ = writer.write_all(
            format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{answer}",
                answer.len()
            )
            .as_bytes(),
        );
    }
}

/// The stand-in's answer to a request for `target` with `headers`.
fn answer(target: &str, headers: &BTreeMap<String, String>, value: &str) -> (&'static str, Value) {
    if headers.get("x-amz-target").map(String::as_str) == Some("secretsmanager.GetSecretValue") {
        (
            "200 OK",
            json!({
                "Name": "prod/portal",
                "SecretString": json!({ "password": value }).to_string(),
                "VersionStages": ["AWSCURRENT"],
            }),
        )
    } else if target.contains("/oauth2/") {
        (
            "200 OK",
            json!({ "token_type": "Bearer", "expires_in": 3599, "access_token": "stand-in-access" }),
        )
    } else if target.ends_with(":access") {
        (
            "200 OK",
            json!({
                "name": "projects/1/secrets/portal/versions/1",
                "payload": { "data": data_encoding::BASE64.encode(value.as_bytes()) },
            }),
        )
    } else if target.starts_with("/v1/secret/data/") {
        (
            "200 OK",
            json!({ "data": { "data": { "password": value }, "metadata": { "version": 1 } } }),
        )
    } else if target.contains("/secrets/") {
        (
            "200 OK",
            json!({ "value": value, "attributes": { "enabled": true } }),
        )
    } else {
        ("404 Not Found", json!({ "errors": [] }))
    }
}

/// A Run whose key-encryption key, a Secret canary, is read through each
/// built-in Secret provider, beside an ops token, an endpoint token canary
/// in the Goodfetch home, starts, beside a call the engine refuses, and
/// finishes; neither canary is in a reply, the engine's stderr, a file of
/// the test's folder (the logs, the Run trace, run state, the Run index,
/// the Checkpoint, the Audit log) or the OTLP export, raw or encoded, but
/// where the test planted it.
///
/// The OS keychain's provider is searched the same way by `secrets.rs`'s
/// test of it, which needs the host's Secret Service.
///
/// Covers: story:4-91, setting:run_state.key_reference, setting:otlp.traces_endpoint
#[rstest]
#[case::env(Provider::Env)]
#[case::secretfile(Provider::SecretFile)]
#[cfg_attr(unix, case::one_password(Provider::OnePassword))]
#[cfg_attr(unix, case::bitwarden(Provider::Bitwarden))]
#[case::vault(Provider::Vault)]
#[case::aws_secrets_manager(Provider::AwsSecretsManager)]
#[case::gcp_secret_manager(Provider::GcpSecretManager)]
#[case::azure_key_vault(Provider::AzureKeyVault)]
fn a_key_read_through_each_provider_and_an_endpoint_token_are_in_no_output(
    #[case] provider: Provider,
) {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let key = canaries.plant(SecretKind::RunDataKey).value().to_owned();
    let key_id = canaries.iter().last().unwrap().id().to_owned();
    let token = canaries.plant(SecretKind::EndpointToken);
    let (token_id, token) = (token.id().to_owned(), token.value().to_owned());
    let ops_token = env.home().join("0.0").join("ops.token");
    std::fs::create_dir_all(env.home().join("0.0")).unwrap();
    std::fs::write(&ops_token, &token).unwrap();
    canaries.record_planted(&token_id, &ops_token.display().to_string());
    let server = stand_in(key.clone());
    let configured = provider.configure(&env, &mut canaries, &key_id, &key, &server);
    let (endpoint, exports, _) =
        crate::log::collector_of::<ExportTraceServiceRequest>("/v1/traces");
    env.user_configuration(&format!(
        "{}otlp.traces_endpoint = \"{endpoint}\"\n",
        configured.configuration
    ));

    let mut engine = Attached::start_with(&env, |command| {
        for (name, value) in &configured.variables {
            command.env(name, value);
        }
    });
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let run = json!({ "run_id": "swept" });
    let mut replies = vec![
        call(&mut engine, 1, "run.start", &run),
        call(
            &mut engine,
            2,
            "run.set_progress_message",
            &json!({ "run_id": "absent", "message": "Reading page 1" }),
        ),
        call(&mut engine, 3, "run.finish", &run),
    ];
    let (rest, stderr) = engine.finish_with_stderr();
    replies.extend(rest);
    let exported: Vec<ExportTraceServiceRequest> = exports.try_iter().collect();

    let replies_bytes = serde_json::to_vec(&replies).unwrap();
    let export_bytes: Vec<u8> = exported
        .iter()
        .flat_map(|export| {
            let mut bytes = export.encode_to_vec();
            bytes.extend_from_slice(format!("{export:?}").as_bytes());
            bytes
        })
        .collect();
    let leaks: Vec<String> = canaries
        .leaks(&[
            Output::Bytes {
                name: "replies",
                bytes: &replies_bytes,
            },
            Output::Bytes {
                name: "stderr",
                bytes: stderr.as_bytes(),
            },
            Output::Bytes {
                name: "OTLP export",
                bytes: &export_bytes,
            },
            Output::Path(env.folder()),
        ])
        .unwrap()
        .into_iter()
        .map(|hit| format!("{} {:?} in {}", hit.canary, hit.encoding, hit.output))
        .collect();
    assert_eq!(
        (
            &replies[0]["result"]["run_id"],
            replies[1]["error"]["message"].is_string(),
            &replies[2]["result"]["lifecycle"],
            !exported.is_empty(),
            leaks,
        ),
        (&json!("swept"), true, &json!("finished"), true, Vec::new()),
        "{replies:?}\n{stderr}"
    );
}

/// The Proving ground's own seed for the Session cookie's test.
const SEED: u64 = 649;

/// An Attached engine's calls, matched to their answers by ID in whatever
/// order those come, and the requests a route's handler is asked about,
/// held until the test lets them go on.
struct Calls<'env> {
    engine: Attached<'env>,
    answers: BTreeMap<i64, Value>,
    held: Vec<Value>,
}

impl<'env> Calls<'env> {
    fn new(engine: Attached<'env>) -> Self {
        Self {
            engine,
            answers: BTreeMap::new(),
            held: Vec::new(),
        }
    }

    /// Sends the call `id`, waiting for no answer.
    fn send(&mut self, id: i64, method: &str, params: &Value) {
        self.engine
            .send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    }

    /// Takes in the engine's next message: an answer, kept by its ID, or a
    /// `route.handle` request, kept unanswered.
    fn read(&mut self) {
        let message = self.engine.next().expect("the engine answered");
        if message["method"] == json!("route.handle") {
            self.held.push(message);
        } else if let Some(id) = message["id"].as_i64() {
            self.answers.insert(id, message);
        }
    }

    /// The answer to the call `id`, whenever it came.
    fn answer(&mut self, id: i64) -> Value {
        loop {
            if let Some(answer) = self.answers.remove(&id) {
                return answer;
            }
            self.read();
        }
    }

    fn call(&mut self, id: i64, method: &str, params: &Value) -> Value {
        self.send(id, method, params);
        self.answer(id)
    }

    /// The first `route.handle` request not yet let go on, once one came.
    fn held(&mut self) -> Value {
        while self.held.is_empty() {
            self.read();
        }
        self.held.remove(0)
    }

    /// Lets the held request `handle` go on as the page sent it, through
    /// the call `id`, whose answer it returns, and ends the handler's turn.
    fn go_on(&mut self, id: i64, handle: &Value) -> Value {
        let answer = self.call(
            id,
            "route.continue",
            &json!({ "route_id": handle["params"]["route_id"] }),
        );
        self.engine
            .send(&json!({ "jsonrpc": "2.0", "id": handle["id"], "result": {} }));
        answer
    }
}

/// Every file and folder of `folder`, at any depth, as few as cover them,
/// but `left_out` and what it holds.
fn paths_but(folder: &Path, left_out: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    let mut paths = Vec::new();
    for entry in entries {
        if entry == left_out {
            continue;
        }
        if left_out.starts_with(&entry) {
            paths.extend(paths_but(&entry, left_out));
        } else {
            paths.push(entry);
        }
    }
    paths
}

/// A login on the Proving ground's `canaries` flow sets a Session cookie,
/// a Secret canary, in a page of the Run's default Session; the browser
/// sends it only where its flow does, and it is in no reply and no file of
/// the test's folder once the Run finished, but the Chrome for Testing
/// build the test copied in, Playwright's own files, which no engine
/// writes.
///
/// Covers: story:4-91
#[test]
fn a_session_cookie_a_login_sets_is_in_no_output() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut canaries = Canaries::new(env.seed());
    let account = canaries
        .plant_through(&proving_ground, SecretKind::SessionCookie)
        .unwrap()
        .account()
        .unwrap()
        .clone();
    let login = proving_ground.start_url("canaries/3").unwrap();
    let mut calls = Calls::new(started(&env, &cache, &proving_ground, "cookie"));

    let page = opened(&calls.call(2, "run.new_page", &json!({ "run_id": "cookie" }))).page_id;
    let mut replies = vec![calls.call(3, "page.goto", &json!({ "page_id": page, "url": login }))];
    for (id, (selector, value)) in (4..).zip([
        ("input[name=username]", account.username.as_str()),
        ("input[name=password]", account.password.as_str()),
    ]) {
        replies.push(calls.call(
            id,
            "locator.fill",
            &json!({ "locator": { "page_id": page, "selector": selector }, "value": value }),
        ));
    }
    // The login form's request is held until the wait for the account page
    // is armed: the account page's call goes once the login's navigation
    // reached that page and loaded it, however early the click answers and
    // however loaded the host (#1764).
    calls.send(
        6,
        "page.route",
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/login" } }),
    );
    calls.send(
        7,
        "locator.click",
        &json!({ "locator": { "page_id": page, "selector": "button[type=submit]" } }),
    );
    let form = calls.held();
    calls.send(
        8,
        "page.wait_for_url",
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/account" } }),
    );
    replies.push(calls.go_on(9, &form));
    replies.push(form);
    replies.push(calls.answer(7));
    replies.push(calls.answer(8));
    let account_page = calls.call(
        10,
        "page.goto",
        &json!({ "page_id": page, "url": login.replace("/login", "/account") }),
    );
    replies.push(account_page.clone());
    replies.push(calls.call(11, "run.finish", &json!({ "run_id": "cookie" })));
    let (rest, _) = calls.engine.finish();
    replies.extend(rest);
    replies.extend(calls.answers.into_values());

    let replies_bytes = serde_json::to_vec(&replies).unwrap();
    let searched = paths_but(env.folder(), &chrome_for_testing_files(&cache));
    let mut outputs = vec![Output::Bytes {
        name: "replies",
        bytes: &replies_bytes,
    }];
    outputs.extend(searched.iter().map(|path| Output::Path(path)));
    let leaks: Vec<String> = canaries
        .leaks(&outputs)
        .unwrap()
        .into_iter()
        .map(|hit| format!("{:?} in {}", hit.encoding, hit.output))
        .collect();
    let sightings: Vec<String> = canaries
        .sightings(&proving_ground)
        .unwrap()
        .into_iter()
        .map(|sighting| {
            format!(
                "{:?} {} {}",
                sighting.placement, sighting.part, sighting.target
            )
        })
        .collect();
    assert_eq!(
        (
            &account_page["result"]["response"]["status"],
            sightings
                .iter()
                .all(|sighting| sighting.starts_with("Planted"))
                && !sightings.is_empty(),
            leaks,
        ),
        (&json!(200), true, Vec::<String>::new()),
        "{sightings:?}\n{replies:?}"
    );
}
