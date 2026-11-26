use std::collections::BTreeSet;

use goodfetch_protocol::plugin::NetworkLevel;
use rstest::rstest;

use super::{AF_UNIX, CLONE_THREAD, Call, Condition, IOCTLS, Rule, TIOCSTI, Verdict, rules};

/// Whether `rule` matches a call whose arguments are all `value`, as the
/// filter compares them.
fn matches(rule: &Rule, value: u64) -> bool {
    match rule.when {
        None => true,
        Some(Condition::Equals { value: wanted, .. }) => value == wanted,
        Some(Condition::Differs { value: other, .. }) => value != other,
        Some(Condition::HasAll { mask, .. }) => value & mask == mask,
        Some(Condition::HasNone { mask, .. }) => value & mask == 0,
    }
}

#[test]
fn each_rule_at_none_has_a_name_of_its_own() {
    let rules = rules(NetworkLevel::None);
    let names: BTreeSet<&str> = rules.iter().map(|rule| rule.name).collect();

    assert_eq!(names.len(), rules.len());
}

#[test]
fn at_none_no_rule_lets_a_socket_be_made() {
    let rules = rules(NetworkLevel::None);

    assert!(
        !rules
            .iter()
            .any(|rule| rule.call == Call::Socket && rule.verdict == Verdict::Allow)
    );
    assert!(rules.iter().any(|rule| rule.call == Call::Socket
        && rule.verdict == Verdict::Refuse
        && rule.when.is_none()));
}

#[rstest]
#[case(Call::IoUringSetup)]
#[case(Call::IoUringEnter)]
#[case(Call::Ptrace)]
#[case(Call::ProcessVmReadv)]
#[case(Call::ProcessVmWritev)]
#[case(Call::Keyctl)]
#[case(Call::Unshare)]
#[case(Call::Setns)]
#[case(Call::Msgget)]
#[case(Call::Semget)]
#[case(Call::Shmget)]
#[case(Call::Listen)]
#[case(Call::Accept)]
#[case(Call::Accept4)]
#[case(Call::Fork)]
#[case(Call::Vfork)]
fn each_call_adr_0033_refuses_has_a_refusal_and_nothing_allowing_it(#[case] call: Call) {
    let rules = rules(NetworkLevel::None);
    let of_call: Vec<&Rule> = rules.iter().filter(|rule| rule.call == call).collect();

    assert_eq!(
        of_call
            .iter()
            .map(|rule| (rule.verdict, rule.when))
            .collect::<Vec<_>>(),
        vec![(Verdict::Refuse, None)],
    );
}

/// The arguments a rule over one call compares: no value of them may be
/// both allowed and refused, which the filters would settle by precedence
/// rather than as the list says.
#[test]
fn no_argument_value_is_both_allowed_and_refused() {
    let rules = rules(NetworkLevel::None);
    let mut values = vec![
        0,
        u64::MAX,
        CLONE_THREAD,
        CLONE_THREAD | 0x11,
        AF_UNIX,
        2,
        TIOCSTI,
    ];
    values.extend(IOCTLS.iter().map(|&(_, request)| request));
    let calls: BTreeSet<Call> = rules.iter().map(|rule| rule.call).collect();

    for call in calls {
        let of_call: Vec<&Rule> = rules.iter().filter(|rule| rule.call == call).collect();
        for &value in &values {
            let verdicts: BTreeSet<_> = of_call
                .iter()
                .filter(|rule| matches(rule, value))
                .map(|rule| format!("{:?}", rule.verdict))
                .collect();
            assert!(
                verdicts.len() <= 1,
                "{call:?} with {value:#x}: {verdicts:?}"
            );
        }
    }
}

#[rstest]
#[case::a_thread(CLONE_THREAD | 0x0d00, Some(Verdict::Allow))]
#[case::a_process(0x11, Some(Verdict::Refuse))]
fn clone_makes_threads_but_no_process(#[case] flags: u64, #[case] verdict: Option<Verdict>) {
    let rules = rules(NetworkLevel::None);
    let found = rules
        .iter()
        .find(|rule| rule.call == Call::Clone && matches(rule, flags))
        .map(|rule| rule.verdict);

    assert_eq!(found, verdict);
}

#[test]
fn clone3_answers_that_it_does_not_exist_so_runtimes_fall_back_to_clone() {
    let rules = rules(NetworkLevel::None);
    let clone3: Vec<Verdict> = rules
        .iter()
        .filter(|rule| rule.call == Call::Clone3)
        .map(|rule| rule.verdict)
        .collect();

    assert_eq!(clone3, vec![Verdict::NotImplemented]);
}

#[test]
fn socketpair_makes_only_unix_sockets() {
    let rules = rules(NetworkLevel::None);
    let verdict = |family| {
        rules
            .iter()
            .find(|rule| rule.call == Call::Socketpair && matches(rule, family))
            .map(|rule| rule.verdict)
    };

    assert_eq!(
        (verdict(AF_UNIX), verdict(2)),
        (Some(Verdict::Allow), Some(Verdict::Refuse))
    );
}

#[rstest]
#[case(NetworkLevel::ProxyTcp)]
#[case(NetworkLevel::Direct)]
fn a_level_not_built_yet_has_no_list(#[case] level: NetworkLevel) {
    assert_eq!(rules(level), Vec::<Rule>::new());
}
