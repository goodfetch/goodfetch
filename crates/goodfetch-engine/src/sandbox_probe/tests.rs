#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;

use goodfetch_protocol::security::{SECURITY_BROWSER_SANDBOX, SandboxEnvironment};
use rstest::rstest;
use serde_json::json;

use super::{
    Facts, Host, NoSandboxPlatform, Probed, SandboxProbe, UserNamespace, host_defaults, judge,
    label_holds_user_namespaces,
};
use crate::settings::HostDefaults;

const EPERM: i32 = 1;

/// A host that refuses the user namespace, where an engine's environment
/// map decides the answer.
fn refusing_host() -> Host {
    Host {
        user_namespace: UserNamespace::Refused(EPERM),
        seccomp_filtered: false,
        apparmor_restricts: false,
    }
}

/// An SDK or a test holds two engines in one process, each started with
/// its own environment map: neither takes the first one's answer.
#[test]
fn two_engines_in_one_process_each_answer_from_their_own_environment() {
    let on_lambda = SandboxProbe::of(
        refusing_host,
        BTreeMap::from([(
            OsString::from("AWS_LAMBDA_FUNCTION_NAME"),
            OsString::from("a-function"),
        )]),
    );
    let elsewhere = SandboxProbe::of(refusing_host, BTreeMap::new());

    assert_eq!(
        on_lambda.probed(),
        &Probed::NoSandbox(NoSandboxPlatform::AwsLambda)
    );
    assert!(
        matches!(elsewhere.probed(), Probed::Unknown { .. }),
        "got {:?}",
        elsewhere.probed()
    );
}

fn facts(user_namespace: UserNamespace) -> Facts {
    Facts {
        user_namespace,
        seccomp_filtered: false,
        apparmor_restricts: false,
        kubernetes: false,
        platform: None,
    }
}

fn environment(probed: &Probed) -> Option<SandboxEnvironment> {
    match probed {
        Probed::Refused { environment, .. } => Some(*environment),
        _ => None,
    }
}

/// The probe's answers: a namespace made starts the sandbox wherever it
/// runs; a seccomp refusal is a container's default profile, a pod's when
/// Kubernetes runs it; AppArmor's restriction leaves the decision to each
/// browser's profile; a platform with no fix switches the sandbox off.
/// Covers: story:4-86
#[rstest]
#[case::a_namespace_made(facts(UserNamespace::Created), "starts", None)]
#[case::a_namespace_made_even_on_lambda(
    Facts { platform: Some(NoSandboxPlatform::AwsLambda), ..facts(UserNamespace::Created) },
    "starts",
    None
)]
#[case::dockers_default_seccomp(
    Facts { seccomp_filtered: true, ..facts(UserNamespace::Refused(EPERM)) },
    "refused",
    Some(SandboxEnvironment::DockerDefaultSeccomp)
)]
#[case::kubernetes_runtime_default(
    Facts { seccomp_filtered: true, kubernetes: true, ..facts(UserNamespace::Refused(EPERM)) },
    "refused",
    Some(SandboxEnvironment::KubernetesRuntimeDefault)
)]
#[case::apparmors_restriction(
    Facts { apparmor_restricts: true, ..facts(UserNamespace::Unmapped(EPERM)) },
    "restricted",
    None
)]
#[case::a_seccomp_filter_beside_apparmors_restriction(
    Facts { seccomp_filtered: true, apparmor_restricts: true, ..facts(UserNamespace::Refused(EPERM)) },
    "refused",
    Some(SandboxEnvironment::DockerDefaultSeccomp)
)]
#[case::lambda(
    Facts { platform: Some(NoSandboxPlatform::AwsLambda), seccomp_filtered: true, ..facts(UserNamespace::Refused(EPERM)) },
    "no_sandbox",
    None
)]
#[case::a_pod_on_eks_on_fargate(
    Facts { platform: Some(NoSandboxPlatform::EksFargate), seccomp_filtered: true, kubernetes: true, ..facts(UserNamespace::Refused(EPERM)) },
    "no_sandbox",
    None
)]
#[case::an_unknown_refusal(facts(UserNamespace::Refused(EPERM)), "unknown", None)]
fn the_probe_names_what_refuses_the_sandbox_and_whether_a_fix_exists(
    #[case] facts: Facts,
    #[case] answer: &str,
    #[case] refused_by: Option<SandboxEnvironment>,
) {
    let probed = judge(&facts);

    let kind = match &probed {
        Probed::Starts => "starts",
        Probed::Restricted => "restricted",
        Probed::Refused { .. } => "refused",
        Probed::NoSandbox(_) => "no_sandbox",
        Probed::Unknown { .. } => "unknown",
    };
    assert_eq!((kind, environment(&probed)), (answer, refused_by));
}

/// A refusal names the environment and its fix, which says how the
/// Operator policy switches the sandbox off instead.
/// Covers: story:4-86, error:browser_sandbox_unavailable
#[test]
fn a_refusal_is_browser_sandbox_unavailable_naming_the_environment_and_its_fix() {
    let probed = judge(&Facts {
        seccomp_filtered: true,
        kubernetes: true,
        ..facts(UserNamespace::Refused(EPERM))
    });

    let refusal = probed.refusal().unwrap();

    let data = refusal.details().unwrap();
    assert_eq!(
        (
            refusal.code(),
            refusal.retryable(),
            data["environment"].clone(),
            data["fix"]
                .as_str()
                .unwrap()
                .contains("securityContext.seccompProfile"),
            data["fix"]
                .as_str()
                .unwrap()
                .contains("`security.browser_sandbox` off in Operator policy"),
        ),
        (
            "browser_sandbox_unavailable",
            false,
            json!("kubernetes_runtime_default"),
            true,
            true
        )
    );
}

/// A refusal the probe does not know, such as `user.max_user_namespaces`
/// at 0, still refuses the launch with the probe's error, so the browser
/// never starts without the sandbox it was asked for: its fix says how to
/// find what refuses the namespace or to switch the sandbox off.
/// Covers: story:4-86, error:browser_sandbox_unavailable
#[test]
fn an_unknown_refusal_refuses_the_launch_with_the_probes_error() {
    let probed = judge(&facts(UserNamespace::Refused(EPERM)));

    let refusal = probed.refusal().unwrap();

    let data = refusal.details().unwrap();
    assert_eq!(
        (
            refusal.code(),
            data["environment"].clone(),
            data["fix"]
                .as_str()
                .unwrap()
                .contains("`security.browser_sandbox` off in Operator policy"),
        ),
        ("browser_sandbox_unavailable", json!("unknown"), true),
        "{}",
        refusal.message()
    );
}

/// A platform with no sandbox still refuses a launch with
/// `browser_sandbox_unavailable` when `security.browser_sandbox` resolves
/// to `true` (for example, locked on in Operator policy), naming the
/// platform in its message and fix.
/// Covers: story:4-86, error:browser_sandbox_unavailable
#[rstest]
#[case::lambda(NoSandboxPlatform::AwsLambda, "AWS Lambda")]
#[case::fargate(NoSandboxPlatform::Fargate, "AWS Fargate")]
#[case::eks_on_fargate(NoSandboxPlatform::EksFargate, "EKS on Fargate")]
#[case::azure_container_apps(NoSandboxPlatform::AzureContainerApps, "Azure Container Apps")]
fn a_no_sandbox_platform_refuses_a_sandboxed_launch_naming_the_platform(
    #[case] platform: NoSandboxPlatform,
    #[case] platform_name: &str,
) {
    let probed = Probed::NoSandbox(platform);

    let refusal = probed.refusal().expect("refused when sandbox stays on");

    let data = refusal.details().unwrap();
    assert_eq!(
        (
            refusal.code(),
            refusal.retryable(),
            refusal.message().contains(platform_name),
            data["environment"].clone(),
            data["fix"].as_str().unwrap().contains(platform_name),
            data["fix"]
                .as_str()
                .unwrap()
                .contains("`security.browser_sandbox`"),
        ),
        (
            "browser_sandbox_unavailable",
            false,
            true,
            json!("unknown"),
            true,
            true,
        ),
        "{}",
        refusal.message()
    );
}

/// On a platform with no fix, the host's default switches the sandbox off;
/// elsewhere the catalog's stays.
/// Covers: story:4-86, setting:security.browser_sandbox
#[rstest]
#[case::lambda(Probed::NoSandbox(NoSandboxPlatform::AwsLambda), json!(false))]
#[case::eks_on_fargate(Probed::NoSandbox(NoSandboxPlatform::EksFargate), json!(false))]
#[case::a_host_whose_sandbox_starts(Probed::Starts, json!(true))]
#[case::apparmors_restriction(Probed::Restricted, json!(true))]
fn a_platform_with_no_sandbox_switches_it_off_by_default(
    #[case] probed: Probed,
    #[case] default: serde_json::Value,
) {
    let defaults = host_defaults(HostDefaults::default(), &probed);

    assert_eq!(defaults.of(&SECURITY_BROWSER_SANDBOX), default);
}

/// The labels AppArmor gives a browser: its own profile lifts the
/// restriction; unconfined, or stacked with `unprivileged_userns`, it is
/// held.
/// Covers: story:4-86
#[rstest]
#[case::unconfined("unconfined\n", true)]
#[case::its_own_profile("goodfetch-chrome-for-testing (unconfined)\n", false)]
#[case::an_enforced_profile("chrome (enforce)", false)]
#[case::stacked_with_the_restriction("unconfined//&unprivileged_userns (enforce)\n", true)]
#[case::its_profile_stacked_with_the_restriction(
    "goodfetch-chrome-for-testing//&unprivileged_userns (enforce)",
    true
)]
fn a_browser_unconfined_or_stacked_with_the_restriction_is_held(
    #[case] label: &str,
    #[case] held: bool,
) {
    assert_eq!(label_holds_user_namespaces(label), held);
}

/// Each platform with no fix is known by the variable it sets, EKS on
/// Fargate by the profile its pod's label names, which the downward API
/// leaves empty on any other node.
/// Covers: story:4-86
#[cfg(target_os = "linux")]
#[rstest]
#[case::lambda("AWS_LAMBDA_FUNCTION_NAME", "shop", Some(NoSandboxPlatform::AwsLambda))]
#[case::fargate(
    "AWS_EXECUTION_ENV",
    "AWS_ECS_FARGATE",
    Some(NoSandboxPlatform::Fargate)
)]
#[case::ecs_on_ec2("AWS_EXECUTION_ENV", "AWS_ECS_EC2", None)]
#[case::eks_on_fargate(
    "GOODFETCH_EKS_FARGATE_PROFILE",
    "shop-profile",
    Some(NoSandboxPlatform::EksFargate)
)]
#[case::eks_on_ec2_where_the_label_is_absent("GOODFETCH_EKS_FARGATE_PROFILE", "", None)]
#[case::azure_container_apps(
    "CONTAINER_APP_NAME",
    "shop",
    Some(NoSandboxPlatform::AzureContainerApps)
)]
#[case::another_host("HOME", "/home/shop", None)]
fn a_platform_with_no_sandbox_is_known_by_its_variable(
    #[case] name: &str,
    #[case] value: &str,
    #[case] platform: Option<NoSandboxPlatform>,
) {
    assert_eq!(
        super::platform(|variable| (variable == name).then(|| value.to_owned())),
        platform
    );
}

struct GeneratedSandboxCase {
    rule: String,
    positive_control: String,
    user_namespace: String,
    seccomp_filtered: bool,
    apparmor_restricts: bool,
    env_var: Option<String>,
    env_value: Option<String>,
    expected_probed: String,
    expected_environment: Option<String>,
    expected_platform: Option<String>,
    expected_error_code: Option<String>,
}

fn load_generated_sandbox_cases() -> Vec<GeneratedSandboxCase> {
    let document: toml_edit::DocumentMut = include_str!("../../probes/browser-sandbox.toml")
        .parse()
        .expect("generated browser-sandbox probe table parses");
    let cases = document["case"]
        .as_array_of_tables()
        .expect("`[[case]]` array present");
    let str_field = |table: &toml_edit::Table, key: &str| {
        table
            .get(key)
            .and_then(|item| item.as_str())
            .map(str::to_owned)
    };
    cases
        .iter()
        .map(|table| GeneratedSandboxCase {
            rule: str_field(table, "rule").expect("rule"),
            positive_control: str_field(table, "positive_control").expect("positive_control"),
            user_namespace: str_field(table, "user_namespace").expect("user_namespace"),
            seccomp_filtered: table["seccomp_filtered"]
                .as_bool()
                .expect("seccomp_filtered"),
            apparmor_restricts: table["apparmor_restricts"]
                .as_bool()
                .expect("apparmor_restricts"),
            env_var: str_field(table, "env_var"),
            env_value: str_field(table, "env_value"),
            expected_probed: str_field(table, "expected_probed").expect("expected_probed"),
            expected_environment: str_field(table, "expected_environment"),
            expected_platform: str_field(table, "expected_platform"),
            expected_error_code: str_field(table, "expected_error_code"),
        })
        .collect()
}

fn starts_host() -> Host {
    Host {
        user_namespace: UserNamespace::Created,
        seccomp_filtered: false,
        apparmor_restricts: false,
    }
}

fn run_generated_case(
    case: &GeneratedSandboxCase,
    positive_control_host: fn() -> Host,
) -> Result<(), String> {
    let mut env = BTreeMap::new();
    if let (Some(var), Some(val)) = (&case.env_var, &case.env_value) {
        env.insert(OsString::from(var), OsString::from(val));
    }
    let positive = SandboxProbe::of(positive_control_host, env.clone());
    if case.positive_control != "starts" || positive.probed() != &Probed::Starts {
        return Err(format!(
            "{}: positive control failed, got {:?}",
            case.rule,
            positive.probed()
        ));
    }
    let host_fn: fn() -> Host = match (
        case.user_namespace.as_str(),
        case.seccomp_filtered,
        case.apparmor_restricts,
    ) {
        ("created", false, false) => starts_host,
        ("refused", true, false) => || Host {
            user_namespace: UserNamespace::Refused(EPERM),
            seccomp_filtered: true,
            apparmor_restricts: false,
        },
        ("refused", false, false) => refusing_host,
        ("unmapped", false, true) => || Host {
            user_namespace: UserNamespace::Unmapped(EPERM),
            seccomp_filtered: false,
            apparmor_restricts: true,
        },
        other => return Err(format!("{}: unhandled host tuple {other:?}", case.rule)),
    };
    let probe = SandboxProbe::of(host_fn, env);
    let probed = probe.probed();
    let (kind, found_env, found_platform) = match probed {
        Probed::Starts => ("starts", None, None),
        Probed::Restricted => ("restricted", None, None),
        Probed::Refused { environment, .. } => {
            ("refused", Some(super::serde_name(*environment)), None)
        }
        Probed::NoSandbox(platform) => (
            "no_sandbox",
            None,
            Some(match platform {
                NoSandboxPlatform::AwsLambda => "aws_lambda",
                NoSandboxPlatform::Fargate => "fargate",
                NoSandboxPlatform::EksFargate => "eks_fargate",
                NoSandboxPlatform::AzureContainerApps => "azure_container_apps",
            }),
        ),
        Probed::Unknown { .. } => ("unknown", None, None),
    };
    if kind != case.expected_probed
        || found_env != case.expected_environment.as_deref()
        || found_platform != case.expected_platform.as_deref()
    {
        return Err(format!(
            "{}: expected ({}, {:?}, {:?}), got ({kind}, {found_env:?}, {found_platform:?})",
            case.rule, case.expected_probed, case.expected_environment, case.expected_platform
        ));
    }
    let found_code = probed.refusal().map(|err| err.code());
    if found_code != case.expected_error_code.as_deref() {
        return Err(format!(
            "{}: expected error code {:?}, got {found_code:?}",
            case.rule, case.expected_error_code
        ));
    }
    Ok(())
}

/// Every browser sandbox startup probe case generated by `xtask probes generate`
/// passes its `starts` positive control first, then produces its exact `Probed`
/// outcome and error code through `SandboxProbe::of` (moved from #642 into #429).
#[test]
fn every_generated_browser_sandbox_startup_probe_passes_with_positive_control() {
    let cases = load_generated_sandbox_cases();
    assert!(!cases.is_empty());
    for case in &cases {
        run_generated_case(case, starts_host).unwrap_or_else(|err| panic!("{err}"));
    }
}

#[test]
fn a_generated_browser_sandbox_probe_fails_when_its_positive_control_fails() {
    let cases = load_generated_sandbox_cases();
    let refused = cases
        .iter()
        .find(|case| case.expected_probed == "refused")
        .expect("refused case present");

    let err = run_generated_case(refused, refusing_host)
        .expect_err("a failing positive control must fail the probe");
    assert!(err.contains("positive control failed"), "{err}");
}
