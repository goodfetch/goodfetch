use rstest::rstest;

use super::{Action, Message, Outcome, WallReason};

/// Each message the runner sends reads as what it asks or reports.
#[rstest]
#[case::a_click(
    r#"{"kind":"click","handle":3,"id":7}"#,
    Message::Click { id: 7, handle: 3 }
)]
#[case::a_main_world_snippet(
    r#"{"kind":"eval","snippet":"EVAL_COOKIEBOT_3","code":"(() => 1)()","id":2}"#,
    Message::Eval { id: 2, snippet: "EVAL_COOKIEBOT_3".to_owned(), code: "(() => 1)()".to_owned() }
)]
#[case::the_hiding_sheet(
    r##"{"kind":"sheet","text":"#a { display: none }","id":1}"##,
    Message::Sheet { id: 1, text: "#a { display: none }".to_owned() }
)]
#[case::a_refusal(
    r#"{"kind":"outcome","url":"https://news.example/","action":"refused","rule":"heuristic","verified":true}"#,
    Message::Outcome(Outcome {
        url: "https://news.example/".to_owned(),
        action: Action::Refused,
        rule: "heuristic".to_owned(),
        verified: Some(true),
        reason: None,
    })
)]
#[case::a_hidden_notice(
    r#"{"kind":"outcome","url":"https://news.example/","action":"hidden","rule":"heuristic"}"#,
    Message::Outcome(Outcome {
        url: "https://news.example/".to_owned(),
        action: Action::Hidden,
        rule: "heuristic".to_owned(),
        verified: None,
        reason: None,
    })
)]
#[case::a_wall_asked_about(
    r#"{"kind":"wall","reason":"covers","id":4}"#,
    Message::Wall { id: 4, reason: WallReason::Covers }
)]
#[case::a_wall_left(
    r#"{"kind":"outcome","url":"https://news.example/","action":"wall","rule":"heuristic","reason":"pays"}"#,
    Message::Outcome(Outcome {
        url: "https://news.example/".to_owned(),
        action: Action::Wall,
        rule: "heuristic".to_owned(),
        verified: None,
        reason: Some(WallReason::Pays),
    })
)]
#[case::a_wall_accepted(
    r#"{"kind":"outcome","url":"https://news.example/","action":"accepted","rule":"heuristic","reason":"covers","verified":true}"#,
    Message::Outcome(Outcome {
        url: "https://news.example/".to_owned(),
        action: Action::Accepted,
        rule: "heuristic".to_owned(),
        verified: Some(true),
        reason: Some(WallReason::Covers),
    })
)]
#[case::a_prompt_detected_in_the_first_pass(r#"{"kind":"detected"}"#, Message::Detected)]
#[case::the_first_pass_settled(r#"{"kind":"settled"}"#, Message::Settled)]
fn each_message_reads_as_what_it_asks_or_reports(#[case] payload: &str, #[case] read: Message) {
    assert_eq!(Message::read(payload), Some(read));
}

/// A payload the runner never sends is no message.
#[rstest]
#[case::not_json("click")]
#[case::an_unknown_kind(r#"{"kind":"accept","id":1}"#)]
#[case::a_click_without_its_handle(r#"{"kind":"click","id":1}"#)]
#[case::a_wall_for_no_reason_the_runner_gives(r#"{"kind":"wall","reason":"looks","id":1}"#)]
fn a_payload_the_runner_never_sends_is_no_message(#[case] payload: &str) {
    assert_eq!(Message::read(payload), None);
}
