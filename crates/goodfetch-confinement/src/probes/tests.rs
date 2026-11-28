use std::path::PathBuf;

use goodfetch_protocol::plugin::NetworkLevel;
use rstest::rstest;

use super::{
    Arg, CallCase, Errno, Mismatch, Outcome, PathCase, Placed, Placeholder, ProbeTable, ProcessArg,
    check, committed,
};
use crate::deny_list::{self, Denied, Host};
use crate::seccomp::{Call, rules};

fn linux_none() -> ProbeTable {
    ProbeTable::read(committed(Host::Linux, NetworkLevel::None).unwrap()).unwrap()
}

fn linux_entries() -> Vec<&'static Denied> {
    deny_list::on(Host::Linux).collect()
}

/// The rule-to-probe check (ADR 0067), in Quick on every OS.
#[test]
fn every_rule_entry_and_file_rule_at_none_on_linux_has_its_probe_and_no_probe_lacks_a_rule() {
    let mismatches = check(&linux_none(), &rules(NetworkLevel::None), &linux_entries());

    assert_eq!(
        mismatches
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        Vec::<String>::new(),
    );
}

#[test]
fn the_socket_rule_is_probed_on_every_family_linux_numbers() {
    let families: Vec<Arg> = linux_none()
        .call
        .into_iter()
        .filter(|case| case.rule == "socket")
        .map(|case| case.args[0])
        .collect();

    assert_eq!(families, (0..46).map(Arg::Number).collect::<Vec<_>>());
}

fn refusal_of_ptrace() -> CallCase {
    CallCase {
        rule: "ptrace".to_owned(),
        call: Call::Ptrace,
        args: vec![Arg::Number(16), Arg::Number(0)],
        unconfined: Outcome::Errno(Errno::Esrch),
        confined: Outcome::Errno(Errno::Eperm),
        pid_namespace: None,
    }
}

fn signal_to_the_runner() -> CallCase {
    CallCase {
        rule: "signals.outside".to_owned(),
        call: Call::Kill,
        args: vec![Arg::Process(ProcessArg::Parent), Arg::Number(0)],
        unconfined: Outcome::Ok,
        confined: Outcome::Errno(Errno::Eperm),
        pid_namespace: Some(Outcome::Errno(Errno::Esrch)),
    }
}

fn read_of_ssh() -> PathCase {
    PathCase {
        rule: "deny.ssh".to_owned(),
        access: super::Access::Read,
        path: Placed::try_from("{home}/.ssh/id_ed25519".to_owned()).unwrap(),
        unconfined: Outcome::Ok,
        landlock: Outcome::Errno(Errno::Eacces),
        fallback: Outcome::Errno(Errno::Enoent),
    }
}

/// Each change breaks the committed table one way, and the check names it.
#[rstest]
#[case::a_rule_without_its_probe(
    |table: &mut ProbeTable| table.call.retain(|case| case.rule != "ptrace"),
    Mismatch::RuleWithoutProbe("ptrace".to_owned()),
)]
#[case::an_entry_without_its_probe(
    |table: &mut ProbeTable| table.path.retain(|case| case.rule != "deny.ssh"),
    Mismatch::RuleWithoutProbe("deny.ssh".to_owned()),
)]
#[case::a_probe_of_a_removed_rule(
    |table: &mut ProbeTable| table.call.push(CallCase { rule: "ptrace.old".to_owned(), ..refusal_of_ptrace() }),
    Mismatch::ProbeWithoutRule("ptrace.old".to_owned()),
)]
#[case::a_probe_of_another_call(
    |table: &mut ProbeTable| table.call.push(CallCase { call: Call::Keyctl, ..refusal_of_ptrace() }),
    Mismatch::OtherCall { rule: "ptrace".to_owned(), call: Call::Keyctl },
)]
#[case::a_refusal_that_expects_success(
    |table: &mut ProbeTable| table.call.push(CallCase { confined: Outcome::Ok, ..refusal_of_ptrace() }),
    Mismatch::WrongOutcome { rule: "ptrace".to_owned(), expected: Outcome::Errno(Errno::Eperm), found: Outcome::Ok },
)]
#[case::a_seccomp_refusal_said_to_differ_in_a_pid_namespace(
    |table: &mut ProbeTable| table.call.push(CallCase { pid_namespace: Some(Outcome::Errno(Errno::Esrch)), ..refusal_of_ptrace() }),
    Mismatch::WrongOutcome { rule: "ptrace".to_owned(), expected: Outcome::Errno(Errno::Eperm), found: Outcome::Errno(Errno::Esrch) },
)]
#[case::the_signal_rule_without_its_probe(
    |table: &mut ProbeTable| table.call.retain(|case| case.rule != "signals.outside"),
    Mismatch::RuleWithoutProbe("signals.outside".to_owned()),
)]
#[case::a_signal_a_pid_namespace_lets_through(
    |table: &mut ProbeTable| table.call.push(CallCase { pid_namespace: None, ..signal_to_the_runner() }),
    Mismatch::WrongOutcome { rule: "signals.outside".to_owned(), expected: Outcome::Errno(Errno::Esrch), found: Outcome::Errno(Errno::Eperm) },
)]
#[case::a_signal_probed_with_another_call(
    |table: &mut ProbeTable| table.call.push(CallCase { call: Call::Ptrace, ..signal_to_the_runner() }),
    Mismatch::OtherCall { rule: "signals.outside".to_owned(), call: Call::Ptrace },
)]
#[case::a_refusal_without_a_positive_control(
    |table: &mut ProbeTable| table.call.push(CallCase { unconfined: Outcome::Errno(Errno::Eperm), ..refusal_of_ptrace() }),
    Mismatch::NoPositiveControl { rule: "ptrace".to_owned() },
)]
#[case::an_entry_probed_outside_it(
    |table: &mut ProbeTable| table.path.push(PathCase {
        path: Placed::try_from("{home}/.sshd/id_ed25519".to_owned()).unwrap(),
        ..read_of_ssh()
    }),
    Mismatch::OutsideEntry { rule: "deny.ssh".to_owned(), path: PathBuf::from("/probe/home/.sshd/id_ed25519") },
)]
#[case::an_entry_that_lets_landlock_read_it(
    |table: &mut ProbeTable| table.path.push(PathCase { landlock: Outcome::Ok, ..read_of_ssh() }),
    Mismatch::WrongOutcome { rule: "deny.ssh".to_owned(), expected: Outcome::Errno(Errno::Eacces), found: Outcome::Ok },
)]
fn the_check_fails_a_table_that_drifts_from_its_rules(
    #[case] change: fn(&mut ProbeTable),
    #[case] expected: Mismatch,
) {
    let mut table = linux_none();
    change(&mut table);

    assert_eq!(
        check(&table, &rules(NetworkLevel::None), &linux_entries()),
        vec![expected]
    );
}

#[rstest]
#[case::under_a_placeholder("{home}/.ssh/id_ed25519", Some(Placeholder::Home), ".ssh/id_ed25519")]
#[case::the_placeholder_itself("{instance}", Some(Placeholder::Instance), "")]
#[case::absolute("/etc/ld.so.cache", None, "/etc/ld.so.cache")]
fn a_probe_path_reads_and_writes_back_as_written(
    #[case] text: &str,
    #[case] base: Option<Placeholder>,
    #[case] rest: &str,
) {
    let placed = Placed::try_from(text.to_owned()).unwrap();

    assert_eq!(
        (placed.base, placed.rest.clone()),
        (base, PathBuf::from(rest))
    );
    assert_eq!(String::from(placed), text);
}

#[rstest]
#[case::relative("home/.ssh")]
#[case::an_unknown_placeholder("{root}/x")]
#[case::an_unclosed_placeholder("{home/.ssh")]
fn a_probe_path_that_names_no_folder_is_refused(#[case] text: &str) {
    assert!(Placed::try_from(text.to_owned()).is_err());
}

#[rstest]
#[case("ok", Outcome::Ok)]
#[case("exit:7", Outcome::Exit(7))]
#[case("EOPNOTSUPP", Outcome::Errno(Errno::Eopnotsupp))]
fn an_outcome_reads_and_writes_back_as_written(#[case] text: &str, #[case] outcome: Outcome) {
    assert_eq!(Outcome::try_from(text.to_owned()), Ok(outcome));
    assert_eq!(outcome.to_string(), text);
}

#[rstest]
#[case("EWHAT")]
#[case("exit:300")]
#[case("Ok")]
fn an_outcome_no_probe_can_give_is_refused(#[case] text: &str) {
    assert!(Outcome::try_from(text.to_owned()).is_err());
}

#[test]
fn only_linux_at_none_has_a_table_so_far() {
    let hosts = [Host::Linux, Host::MacOs, Host::Windows];
    let levels = [
        NetworkLevel::None,
        NetworkLevel::ProxyTcp,
        NetworkLevel::Direct,
    ];
    let with_table: Vec<(Host, NetworkLevel)> = hosts
        .iter()
        .flat_map(|&host| levels.iter().map(move |&level| (host, level)))
        .filter(|&(host, level)| committed(host, level).is_some())
        .collect();

    assert_eq!(
        with_table,
        vec![
            (Host::Linux, NetworkLevel::None),
            (Host::Linux, NetworkLevel::ProxyTcp),
            (Host::Linux, NetworkLevel::Direct),
        ]
    );
}

#[test]
fn every_rule_entry_and_file_rule_at_proxy_tcp_on_linux_has_its_probe_and_no_probe_lacks_a_rule() {
    let table = ProbeTable::read(committed(Host::Linux, NetworkLevel::ProxyTcp).unwrap()).unwrap();
    let rules = crate::seccomp::rules(NetworkLevel::ProxyTcp);
    let denied: Vec<_> = crate::deny_list::on(Host::Linux).collect();

    assert_eq!(check(&table, &rules, &denied), Vec::new());
}

#[test]
fn every_rule_entry_and_file_rule_at_direct_on_linux_has_its_probe_and_no_probe_lacks_a_rule() {
    let table = ProbeTable::read(committed(Host::Linux, NetworkLevel::Direct).unwrap()).unwrap();
    let rules = crate::seccomp::rules(NetworkLevel::Direct);
    let denied: Vec<_> = crate::deny_list::on(Host::Linux).collect();

    assert_eq!(check(&table, &rules, &denied), Vec::new());
}
