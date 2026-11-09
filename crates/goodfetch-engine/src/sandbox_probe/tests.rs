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
/// never starts without the sandbox it was asked for: its message says what
/// the probe met, and its fix how to find what refuses the namespace or to
/// switch the sandbox off.
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
            refusal
                .message()
                .contains("creating a user namespace was refused"),
            data["fix"]
                .as_str()
                .unwrap()
                .contains("`security.browser_sandbox` off in Operator policy"),
        ),
        ("browser_sandbox_unavailable", json!("unknown"), true, true),
        "{}",
        refusal.message()
    );
}

/// On a platform with no fix, the host's default switches the sandbox off;
/// elsewhere the catalog's stays.
/// Covers: story:4-86, setting:security.browser_sandbox
#[rstest]
#[case::lambda(Probed::NoSandbox(NoSandboxPlatform::AwsLambda), json!(false))]
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

/// Each platform with no fix is known by the variable it sets.
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
