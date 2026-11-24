//! The online copy's deployment (#168; ADR 0032), public in
//! `proving-ground/deploy/`: the workflow deploys only from a release tag,
//! under the protected environment whose reviewer is the owner, and passes
//! the copy each secret its configuration names from that environment; the
//! committed configuration starts the copy on its public origin, and the
//! tunnel brings that origin's requests to the address the copy binds.

use std::path::{Path, PathBuf};

use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{HOST, challenge_tasks, online_arguments};
use crate::proving_ground::ProvingGround;

/// The protected GitHub environment holding the vendors' keys.
const ENVIRONMENT: &str = "proving-ground-online";

/// A file of `proving-ground/deploy/`.
fn deploy_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("deploy")
        .join(name)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The deployment's workflow.
fn workflow() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.github/workflows/proving-ground-online.yml");
    serde_saphyr::from_str(&read(&path)).expect("the workflow is YAML")
}

/// The configuration the deployment serves.
fn deployed_config() -> String {
    read(&deploy_file("online.toml"))
}

/// Every `secret_env` the deployed configuration names.
fn secret_names() -> Vec<String> {
    let config: toml::Table =
        toml::from_str(&deployed_config()).expect("the configuration is TOML");
    config
        .values()
        .filter_map(|section| section.get("secret_env")?.as_str())
        .map(str::to_owned)
        .collect()
}

/// The name of the environment a job runs under, if any.
fn environment(job: &Value) -> Option<&str> {
    job["environment"]
        .as_str()
        .or_else(|| job["environment"]["name"].as_str())
}

/// Covers: story:2-47
#[test]
fn the_online_copy_deploys_only_from_a_release_tag_under_the_protected_environment() {
    let workflow = workflow();

    assert_eq!(
        workflow["on"],
        json!({ "push": { "tags": ["v[0-9]+.[0-9]+.[0-9]+", "v[0-9]+.[0-9]+.[0-9]+-rc.[0-9]+"] } })
    );
    let jobs = workflow["jobs"].as_object().expect("`jobs` is a map");
    assert!(
        jobs.values()
            .any(|job| environment(job) == Some(ENVIRONMENT))
    );
    for (id, job) in jobs {
        let reads_secrets = job.to_string().contains("secrets.");
        assert!(
            !reads_secrets || environment(job) == Some(ENVIRONMENT),
            "the job `{id}` reads secrets outside the protected environment"
        );
    }
}

/// Covers: story:2-47
#[test]
fn every_secret_the_deployed_configuration_names_comes_from_the_protected_environment() {
    let workflow = workflow();
    let passed: Vec<(&str, &str)> = workflow["jobs"]
        .as_object()
        .expect("`jobs` is a map")
        .values()
        .filter(|job| environment(job) == Some(ENVIRONMENT))
        .flat_map(|job| job["steps"].as_array().into_iter().flatten())
        .filter_map(|step| step["env"].as_object())
        .flatten()
        .filter_map(|(name, value)| Some((name.as_str(), value.as_str()?)))
        .collect();
    let names = secret_names();

    assert_ne!(names, Vec::<String>::new());
    for name in &names {
        let from_environment = format!("${{{{ secrets.{name} }}}}");
        assert!(
            passed.contains(&(name.as_str(), from_environment.as_str())),
            "the deploy passes no `{name}` from the environment's secrets"
        );
    }
}

/// Covers: story:2-38, story:2-48
#[test]
fn the_deployed_configuration_starts_every_task_on_the_public_origin() {
    let env = TestEnv::new();
    let config = deployed_config();
    let origin =
        toml::from_str::<toml::Table>(&config).expect("the configuration is TOML")["origin"]
            .as_str()
            .expect("the configuration names its public origin")
            .to_owned();
    let names = secret_names();
    let secrets: Vec<(&str, &str)> = names
        .iter()
        .map(|name| (name.as_str(), "a-secret"))
        .collect();
    let proving_ground = ProvingGround::start_with_env(&online_arguments(&env, &config), &secrets);

    let tasks = challenge_tasks(&proving_ground);

    assert!(origin.starts_with("https://"), "{origin}");
    assert_ne!(tasks, Vec::<(String, String)>::new());
    for (id, start_url) in tasks {
        assert!(
            start_url.starts_with(&format!("{origin}/")),
            "{id}: {start_url}"
        );
    }
}

/// The value of `argument` in the `ExecStart` of the systemd unit `unit`.
fn unit_argument(unit: &str, argument: &str) -> String {
    let text = read(&deploy_file(unit));
    let command = text
        .lines()
        .find_map(|line| line.strip_prefix("ExecStart="))
        .expect("the unit has an ExecStart");
    let mut words = command.split_whitespace();
    words.find(|word| *word == argument);
    words.next().expect("the argument has a value").to_owned()
}

/// The value of `variable` in an `Environment=` line of the systemd unit
/// `unit`.
fn unit_environment(unit: &str, variable: &str) -> String {
    read(&deploy_file(unit))
        .lines()
        .filter_map(|line| line.strip_prefix("Environment="))
        .find_map(|setting| setting.strip_prefix(&format!("{variable}=")))
        .expect("the unit sets the variable")
        .to_owned()
}

/// Covers: story:2-48
#[test]
fn the_tunnel_brings_the_public_host_to_the_copy_and_anubiss_paths_through_anubis() {
    let tunnel: Value =
        serde_saphyr::from_str(&read(&deploy_file("cloudflared.yml"))).expect("the tunnel is YAML");
    let copy = format!(
        "http://{}",
        unit_argument("goodfetch-proving-ground.service", "--address")
    );
    let anubis = format!(
        "http://{}",
        unit_environment("goodfetch-anubis.service", "BIND")
    );

    assert_eq!(
        tunnel["ingress"],
        json!([
            {
                "hostname": "challenges.goodfetch.dev",
                "path": "^/(anubis|\\.within\\.website/)",
                "service": anubis,
                "originRequest": { "httpHostHeader": HOST },
            },
            {
                "hostname": "challenges.goodfetch.dev",
                "service": copy,
                "originRequest": { "httpHostHeader": HOST },
            },
            { "service": "http_status:404" },
        ])
    );
    assert_eq!(unit_environment("goodfetch-anubis.service", "TARGET"), copy);
}
