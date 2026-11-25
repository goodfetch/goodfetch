//! The Walls' items answered by `needs-you answer` (ADRs 0048 and 0052):
//! a consent wall is accepted for its domain or for every domain, and a
//! domain whose refetch stopped is removed from the layer that lists it or
//! denied with a reason. Each writes the settings file its operation names
//! and leaves the list.

use std::path::{Path, PathBuf};

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{PtyRun, TestEnv, cli};
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{COLLIE, NOTHING, QUIET, WHICH_ANSWER, WHICH_ITEM, answering, kinds_listed, talk};
use crate::usage::refusal;

/// The domain whose pages showed the Walls.
const WALLED: &str = "walled.test";

/// What the engine says a person takes on, in each warning once.
const TAKEN_ON: &str = "under the GDPR that consent is yours";

/// The line each answer shows.
const LEFT: &str = "Left the list: 1.";

/// A Run an Attached engine started and left, with the Run index's
/// statement `seen`, which says what it met on `WALLED`.
fn met(env: &TestEnv, seen: &str) {
    let started = cli(env, COLLIE)
        .args(["run", "start", "--run-id", "met", "--attached"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{started}");
    Connection::open(HomeLayout::of(env.home()).run_index())
        .unwrap()
        .execute(seen, (WALLED, "met"))
        .unwrap();
}

/// The consent wall of `WALLED` a Run met.
fn consent_wall(env: &TestEnv) {
    met(
        env,
        "INSERT INTO consent_walls (domain, run_id, seen_at) VALUES (?1, ?2, '2026-01-05T10:00:00Z')",
    );
}

/// The refetch that stopped on `WALLED`, which the project configuration
/// `file` allowed, when there is one.
fn refetch_stopped(env: &TestEnv, file: Option<&Path>) {
    let file = file.map_or("NULL".to_owned(), |file| {
        format!("'{}'", file.display().to_string().replace('\'', "''"))
    });
    met(
        env,
        &format!(
            "INSERT INTO refetch_stops (domain, run_id, cause, project_file, stopped_at) VALUES (?1, ?2, 'challenge', {file}, '2026-01-05T10:00:00Z')"
        ),
    );
}

/// What the settings file `file` sets `key` to, in its table `settings`.
fn set_in(file: &Path, key: &str) -> Value {
    let text = std::fs::read_to_string(file).unwrap();
    let parsed: toml::Table = text.parse().unwrap();
    let mut value = &toml::Value::Table(parsed);
    for part in ["settings"].into_iter().chain(key.split('.')) {
        match value.get(part) {
            Some(inner) => value = inner,
            None => return Value::Null,
        }
    }
    json!(value)
}

/// Waits for the item to leave and ends the session, which says what it
/// printed.
fn left(mut session: PtyRun<'_>) -> String {
    session.expect(LEFT).unwrap();
    session.expect(NOTHING).unwrap();
    let ended = session.finish().unwrap();
    let printed = ended.stripped();
    assert_eq!(ended.exit, Some(0), "{printed}");
    printed
}

/// The Operator policy's file of `env`'s home.
fn operator_policy(env: &TestEnv) -> PathBuf {
    HomeLayout::of(env.home()).operator_policy()
}

/// User configuration's file of `env`'s home.
fn user_configuration(env: &TestEnv) -> PathBuf {
    HomeLayout::of(env.home()).user_configuration()
}

/// A person accepts a consent wall for its domain, or for every domain: the
/// warning is shown once, one question asked, user configuration lists what
/// they accepted, and the item leaves the list.
///
/// Covers: story:12-26, story:12-139
#[rstest::rstest]
#[case::its_domain(
    "1",
    "Accepting the consent walls of walled.test gives consent in your name",
    "Accept the consent walls of walled.test in every later run? [y/N]",
    WALLED
)]
#[case::every_domain(
    "2",
    "Accepting consent walls on every domain gives consent in your name",
    "Accept consent walls on every domain in every later run? [y/N]",
    "*"
)]
fn a_person_accepts_a_consent_wall_from_the_list(
    #[case] answer: &str,
    #[case] warning: &str,
    #[case] question: &str,
    #[case] accepted: &str,
) {
    let env = TestEnv::new();
    env.user_configuration(QUIET);
    consent_wall(&env);
    let mut session = answering(&env, &["--attached"]);

    talk(&mut session, &[(WHICH_ITEM, "1"), (WHICH_ANSWER, answer)]);
    session.expect(warning).unwrap();
    talk(&mut session, &[(question, "y")]);
    let printed = left(session);

    assert_eq!(
        (
            printed.matches(warning).count(),
            set_in(&user_configuration(&env), "consent.domains"),
            kinds_listed(&env),
        ),
        (1, json!([accepted]), vec![]),
        "{printed}"
    );
}

/// A no accepts nothing: the warning was shown, the item stays listed.
///
/// Covers: story:12-26
#[test]
fn a_person_leaves_a_consent_wall_as_it_is() {
    let env = TestEnv::new();
    env.user_configuration(QUIET);
    consent_wall(&env);
    let mut session = answering(&env, &["--attached"]);

    talk(&mut session, &[(WHICH_ITEM, "1"), (WHICH_ANSWER, "1")]);
    session.expect(TAKEN_ON).unwrap();
    talk(
        &mut session,
        &[
            ("in every later run? [y/N]", "n"),
            ("Nothing was changed.", ""),
        ],
    );
    let ended = session.finish().unwrap();

    assert_eq!(
        (
            ended.exit,
            set_in(&user_configuration(&env), "consent.domains"),
            kinds_listed(&env),
        ),
        (Some(0), Value::Null, vec!["consent_wall".to_owned()]),
        "{}",
        ended.stripped()
    );
}

/// A machine's layer of `wall.refetch_domains`: its file, and what writes
/// the domains it lists.
struct Layer {
    file: fn(&TestEnv) -> PathBuf,
    list: fn(&TestEnv, &str),
}

const USER: Layer = Layer {
    file: user_configuration,
    list: |env, domains| {
        env.user_configuration(&format!("{QUIET}wall.refetch_domains = {domains}\n"));
    },
};

const OPERATOR: Layer = Layer {
    file: operator_policy,
    list: |env, domains| {
        env.user_configuration(QUIET);
        std::fs::write(
            operator_policy(env),
            format!("version = \"0.1\"\n\n[settings]\nwall.refetch_domains = {domains}\n"),
        )
        .unwrap();
    },
};

/// A person removes a domain whose refetch stopped from the layer that lists
/// it: user configuration, the Operator policy, or either's `*`, replaced by
/// the domains they type, or by none.
///
/// Covers: story:12-27, story:12-139
#[rstest::rstest]
#[case::user_configuration(USER, r#"["walled.test", "other.test"]"#, None, json!(["other.test"]))]
#[case::operator_policy(OPERATOR, r#"["walled.test"]"#, None, json!([]))]
#[case::star_replaced(
    USER,
    r#"["*"]"#,
    Some("other.test, third.test"),
    json!(["other.test", "third.test"])
)]
#[case::star_removed(OPERATOR, r#"["*"]"#, Some(""), json!([]))]
fn a_person_removes_a_domain_whose_refetch_stopped(
    #[case] layer: Layer,
    #[case] listed: &str,
    #[case] typed: Option<&str>,
    #[case] kept: Value,
) {
    let env = TestEnv::new();
    (layer.list)(&env, listed);
    refetch_stopped(&env, None);
    let mut session = answering(&env, &["--attached"]);

    talk(&mut session, &[(WHICH_ITEM, "1"), (WHICH_ANSWER, "1")]);
    if let Some(typed) = typed {
        talk(
            &mut session,
            &[(
                "The domains that replace `*`, separated by commas, or Enter for none:",
                typed,
            )],
        );
    }
    talk(&mut session, &[("? [y/N]", "y")]);
    let printed = left(session);

    assert_eq!(
        (
            set_in(&(layer.file)(&env), "wall.refetch_domains"),
            kinds_listed(&env),
        ),
        (kept, vec![]),
        "{printed}"
    );
}

/// A domain from project configuration is listed with its file, and offers
/// no removal: its denial and its dismissal are what is left, and it leaves
/// the list when the person dismisses it.
///
/// Covers: story:12-27, story:12-139
#[test]
fn a_domain_from_project_configuration_shows_its_file_and_offers_no_removal() {
    let env = TestEnv::new();
    env.user_configuration(QUIET);
    let project = env.home().join("project").join("goodfetch.toml");
    std::fs::create_dir_all(project.parent().unwrap()).unwrap();
    std::fs::write(
        &project,
        "[settings]\nwall.refetch_domains = [\"walled.test\"]\n",
    )
    .unwrap();
    refetch_stopped(&env, Some(&project));
    let mut session = answering(&env, &["--attached"]);

    session
        .expect("Answers: Deny the domain; Dismiss it.")
        .unwrap();
    session
        .expect(&format!(
            "To do: change `wall.refetch_domains` in the project's `{}` to stop refetching walled.test.",
            project.display()
        ))
        .unwrap();
    talk(
        &mut session,
        &[
            (WHICH_ITEM, "1"),
            (WHICH_ANSWER, "2"),
            ("Dismiss this item? It does not come back. [y/N]", "y"),
        ],
    );
    let printed = left(session);

    assert_eq!(kinds_listed(&env), Vec::<String>::new(), "{printed}");
}

/// A denial asks for its reason: none is refused and denies nothing, and
/// one lists the domain in the Operator policy's `wall.denied_domains`.
///
/// Covers: story:12-27, story:12-139
#[test]
fn a_denial_asks_for_its_reason() {
    let env = TestEnv::new();
    (USER.list)(&env, r#"["walled.test"]"#);
    refetch_stopped(&env, None);
    let mut session = answering(&env, &["--attached"]);

    talk(
        &mut session,
        &[
            (WHICH_ITEM, "1"),
            (WHICH_ANSWER, "2"),
            ("Why it is denied", ""),
        ],
    );
    session.expect("A denial needs its reason").unwrap();
    let denied_without_reason = operator_policy(&env).exists();
    talk(
        &mut session,
        &[
            (WHICH_ITEM, "1"),
            (WHICH_ANSWER, "2"),
            ("Why it is denied", "its operator objected"),
            ("Deny walled.test in the Operator policy", "y"),
        ],
    );
    let printed = left(session);

    assert_eq!(
        (
            denied_without_reason,
            set_in(&operator_policy(&env), "wall.denied_domains"),
            kinds_listed(&env),
        ),
        (false, json!([WALLED]), vec![]),
        "{printed}"
    );
}

/// Where no person answers, a consent wall and a stopped refetch are refused
/// with `person_required`, exit `4`, and both stay listed, no file written.
///
/// Covers: story:12-26, story:12-27, story:12-140, exit:refused
#[rstest::rstest]
#[case::under_json(true, Some("1\n"))]
#[case::from_a_pipe(false, Some("1\n"))]
#[case::from_a_script(false, None)]
fn a_walls_answer_is_refused_where_no_person_answers(
    #[case] json: bool,
    #[case] stdin: Option<&str>,
    #[values("consent_wall:walled.test", "wall_refetch_stopped:walled.test")] item: &str,
) {
    let env = TestEnv::new();
    (USER.list)(&env, r#"["walled.test"]"#);
    match item.split_once(':').unwrap().0 {
        "consent_wall" => consent_wall(&env),
        _ => refetch_stopped(&env, None),
    }
    let before = std::fs::read(user_configuration(&env)).unwrap();

    let mut asked = cli(&env, COLLIE).args(["needs-you", "answer", item]);
    if json {
        asked = asked.arg("--json");
    }
    if let Some(stdin) = stdin {
        asked = asked.stdin(stdin);
    }
    let refused = asked.run().unwrap();

    let (code, retryable, _) = refusal(&refused, json);
    assert_eq!(
        (
            refused.exit,
            code,
            retryable,
            kinds_listed(&env),
            std::fs::read(user_configuration(&env)).unwrap() == before,
            operator_policy(&env).exists(),
        ),
        (
            Some(4),
            json!("person_required"),
            json!(false),
            vec![item.split_once(':').unwrap().0.to_owned()],
            true,
            false,
        ),
        "{refused}"
    );
}
