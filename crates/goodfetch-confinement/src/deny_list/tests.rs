use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rstest::rstest;

use super::{Base, DENY_LIST, Denied, Host, Places, on};

fn places() -> Places {
    Places {
        home: PathBuf::from("/home/ada"),
        goodfetch_home: PathBuf::from("/home/ada/.local/share/goodfetch"),
        app_data: None,
        local_app_data: None,
    }
}

fn entry(rule: &str) -> &'static Denied {
    DENY_LIST.iter().find(|denied| denied.rule == rule).unwrap()
}

#[test]
fn each_entry_has_a_rule_name_of_its_own() {
    let names: BTreeSet<&str> = DENY_LIST.iter().map(|denied| denied.rule).collect();

    assert_eq!(names.len(), DENY_LIST.len());
}

#[rstest]
#[case::the_folder_itself("deny.ssh", "/home/ada/.ssh")]
#[case::a_key_inside_it("deny.ssh", "/home/ada/.ssh/id_ed25519")]
#[case::the_home_that_holds_it("deny.ssh", "/home/ada")]
#[case::the_root_that_holds_everything("deny.aws", "/")]
#[case::a_nested_file("deny.docker_config", "/home/ada/.docker/config.json")]
#[case::another_plugin_s_folder(
    "deny.goodfetch_home",
    "/home/ada/.local/share/goodfetch/plugins/other"
)]
#[case::a_dot_env_file("deny.env_file", "/srv/app/.env")]
#[case::a_dot_env_variant("deny.env_file", "/srv/app/.env.production")]
#[case::a_hook("deny.git_hooks", "/srv/app/.git/hooks/pre-commit")]
fn granting_a_path_that_is_holds_or_lies_in_an_entry_grants_it(
    #[case] rule: &str,
    #[case] path: &str,
) {
    assert!(entry(rule).covers(Path::new(path), &places()));
}

#[rstest]
#[case::a_sibling_folder("deny.ssh", "/home/ada/.sshd")]
#[case::another_file_of_the_folder_s_parent("deny.docker_config", "/home/ada/.docker/cli-plugins")]
#[case::a_name_that_only_starts_alike("deny.env_file", "/srv/app/.envrc")]
#[case::an_example_file_named_after_it("deny.env_file", "/srv/app/env.example")]
#[case::the_repository_s_other_files("deny.git_hooks", "/srv/app/.git/config")]
#[case::a_folder_only_named_hooks("deny.git_hooks", "/srv/app/hooks")]
fn a_path_beside_an_entry_is_not_covered(#[case] rule: &str, #[case] path: &str) {
    assert!(!entry(rule).covers(Path::new(path), &places()));
}

#[test]
fn a_windows_entry_resolves_to_nothing_on_a_host_without_its_folder() {
    let denied = entry("deny.credentials_roaming");

    assert_eq!(denied.resolve(&places()), None);
    assert!(!denied.covers(Path::new("/"), &places()));
}

#[test]
fn a_windows_entry_resolves_under_its_folder() {
    let mut places = places();
    places.app_data = Some(PathBuf::from(r"C:\Users\ada\AppData\Roaming"));

    assert_eq!(
        entry("deny.credentials_roaming").resolve(&places),
        Some(
            PathBuf::from(r"C:\Users\ada\AppData\Roaming")
                .join("Microsoft")
                .join("Credentials")
        ),
    );
}

#[test]
fn an_entry_anywhere_resolves_to_no_one_path() {
    assert_eq!(entry("deny.env_file").base, Base::Anywhere);
    assert_eq!(entry("deny.env_file").resolve(&places()), None);
}

#[rstest]
#[case::linux(Host::Linux, "deny.chrome", "deny.keychains")]
#[case::mac_os(Host::MacOs, "deny.keychains", "deny.chrome")]
#[case::windows(Host::Windows, "deny.credentials_local", "deny.bashrc")]
fn each_os_takes_its_own_entries_and_those_it_shares(
    #[case] host: Host,
    #[case] own: &str,
    #[case] other_os: &str,
) {
    let rules: BTreeSet<&str> = on(host).map(|denied| denied.rule).collect();

    assert!(rules.contains(own));
    assert!(!rules.contains(other_os));
    assert!(rules.contains("deny.goodfetch_home"));
    assert!(rules.contains("deny.env_file"));
}
