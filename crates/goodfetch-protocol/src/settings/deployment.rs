//! The deployment variables (ADRs 0059, 0083 and 0090): the closed list of
//! `GOODFETCH_` variables and the name of each, which `settings` exports.

/// The deployment variables (ADRs 0059, 0083 and 0090): the closed list of
/// `GOODFETCH_` variables, each a value that differs by machine, read from
/// the environment map and never a setting. Both engine-path variables are
/// on it, so neither is unknown to either Product; each SDK and crate reads
/// only its own Product's. `GOODFETCH_DAEMON_ADDRESS`,
/// `GOODFETCH_DAEMON_DRAIN_GRACE` and `GOODFETCH_OPS_ADDRESS` are the
/// foreground Daemon's own process configuration (ADRs 0049, 0050 and
/// 0083), which only `daemon run` reads. `GOODFETCH_CHANNEL` is the channel
/// an executable came from, which that channel's launcher or image sets
/// (ADRs 0087 and 0090).
pub const DEPLOYMENT_VARIABLES: &[&str] = &[
    BROWSERS_PATH_VARIABLE,
    CHANNEL_VARIABLE,
    COLLIE_ENGINE_PATH_VARIABLE,
    DAEMON_ADDRESS_VARIABLE,
    DAEMON_DRAIN_GRACE_VARIABLE,
    EKS_FARGATE_PROFILE_VARIABLE,
    FOXHOUND_ENGINE_PATH_VARIABLE,
    HOME_VARIABLE,
    OPS_ADDRESS_VARIABLE,
    TEST_SEED_VARIABLE,
    TOKEN_VARIABLE,
    URL_VARIABLE,
];

/// The Browser cache's folder, in place of `browsers` in Goodfetch home
/// (ADR 0098).
pub const BROWSERS_PATH_VARIABLE: &str = "GOODFETCH_BROWSERS_PATH";

/// The channel an executable came from (ADRs 0087 and 0090).
pub const CHANNEL_VARIABLE: &str = "GOODFETCH_CHANNEL";

/// The channels an executable can come from, by the names
/// `GOODFETCH_CHANNEL` and a release build's stamp give them (ADR 0090).
/// The engine and each SDK refuse any other name.
pub const CHANNELS: &[&str] = &["npm", "pypi", "crates-io", "homebrew", "oci", "installer"];

/// The path of Collie's executable an SDK starts as its Attached engine.
pub const COLLIE_ENGINE_PATH_VARIABLE: &str = "GOODFETCH_COLLIE_ENGINE_PATH";

/// The address the foreground Daemon listens at, over `daemon.address`
/// (ADR 0049).
pub const DAEMON_ADDRESS_VARIABLE: &str = "GOODFETCH_DAEMON_ADDRESS";

/// How long the foreground Daemon drains, over `daemon.drain_grace` (ADR
/// 0049).
pub const DAEMON_DRAIN_GRACE_VARIABLE: &str = "GOODFETCH_DAEMON_DRAIN_GRACE";

/// The Fargate profile EKS labels a pod it runs on Fargate with
/// (`eks.amazonaws.com/fargate-profile`), which the pod passes through the
/// downward API: Chromium's sandbox probe reads EKS on Fargate from it
/// (ADR 0049). On a node of any other kind the label is absent and the
/// value empty, so this variable alone may be empty.
pub const EKS_FARGATE_PROFILE_VARIABLE: &str = "GOODFETCH_EKS_FARGATE_PROFILE";

/// The path of Foxhound's executable an SDK starts as its Attached engine.
pub const FOXHOUND_ENGINE_PATH_VARIABLE: &str = "GOODFETCH_FOXHOUND_ENGINE_PATH";

/// The folder of Goodfetch home (ADRs 0059 and 0090).
pub const HOME_VARIABLE: &str = "GOODFETCH_HOME";

/// The address of the foreground Daemon's ops listener (ADR 0050).
pub const OPS_ADDRESS_VARIABLE: &str = "GOODFETCH_OPS_ADDRESS";

/// The seed of a test's random choices (ADR 0059).
pub const TEST_SEED_VARIABLE: &str = "GOODFETCH_TEST_SEED";

/// The token of the remote Daemon `GOODFETCH_URL` names, which nothing
/// prints (ADRs 0049 and 0083).
pub const TOKEN_VARIABLE: &str = "GOODFETCH_TOKEN";

/// The remote Daemon a command reaches instead of its major's (ADRs 0049
/// and 0083).
pub const URL_VARIABLE: &str = "GOODFETCH_URL";

/// The prefix every deployment variable has, which ADR 0034 reserves.
pub const DEPLOYMENT_PREFIX: &str = "GOODFETCH_";
