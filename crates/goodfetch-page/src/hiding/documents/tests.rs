use goodfetch_cdp::SessionId as TargetSession;

use super::{Documents, Key, Part, Write};

fn key(session: &str, frame: &str) -> Key {
    (TargetSession(session.to_owned()), frame.to_owned())
}

/// The write `documents` asks for to set `part` of `key` to `text`, gone
/// through with the sheet `sheet`.
fn set(documents: &mut Documents, key: &Key, part: Part, text: &str, sheet: &str) -> Write {
    let write = documents
        .set(key, part, text.to_owned())
        .expect("the change needs a write");
    documents.written(&write, sheet.to_owned());
    write
}

/// Live hiding's rules and the Consent prompt runner's share one sheet per
/// document: the sheet is made once, on the target's first write with its
/// domains enabled, and every later change sets the whole text of both
/// parts in it.
#[test]
fn both_parts_of_a_document_share_one_sheet_made_once() {
    let mut documents = Documents::default();
    let main = key("page", "main");
    documents.navigated(&main.0, &main.1, "loader-1");
    assert_eq!(documents.release(&main, None), None);
    let first = set(
        &mut documents,
        &main,
        Part::Hiding,
        ".ad { display: none }",
        "s1",
    );
    let second = set(
        &mut documents,
        &main,
        Part::Consent,
        "#cmp { opacity: 0 }",
        "s1",
    );
    assert_eq!(
        (first, second),
        (
            Write {
                key: main.clone(),
                enable: true,
                sheet: None,
                text: ".ad { display: none }".to_owned(),
            },
            Write {
                key: main.clone(),
                enable: false,
                sheet: Some("s1".to_owned()),
                text: ".ad { display: none }\n#cmp { opacity: 0 }".to_owned(),
            },
        )
    );
}

/// A change that leaves the whole text as the sheet holds it, and an empty
/// part of a document with no sheet, need no command.
#[test]
fn a_change_that_leaves_the_text_needs_no_write() {
    let mut documents = Documents::default();
    let main = key("page", "main");
    assert_eq!(documents.set(&main, Part::Consent, String::new()), None);
    set(&mut documents, &main, Part::Hiding, ".ad {}", "s1");
    assert_eq!(
        documents.set(&main, Part::Hiding, ".ad {}".to_owned()),
        None
    );
}

/// A new document starts with no sheet and no rule; the same document
/// reported again, by another of the page's listeners, keeps both.
#[test]
fn a_new_document_starts_with_no_sheet_and_the_same_one_keeps_its_own() {
    let mut documents = Documents::default();
    let main = key("page", "main");
    documents.navigated(&main.0, &main.1, "loader-1");
    documents.release(&main, None);
    set(&mut documents, &main, Part::Hiding, ".ad {}", "s1");
    documents.navigated(&main.0, &main.1, "loader-1");
    let kept = documents.set(&main, Part::Consent, "#cmp {}".to_owned());
    documents.navigated(&main.0, &main.1, "loader-2");
    let fresh = documents.set(&main, Part::Consent, "#cmp {}".to_owned());
    assert_eq!(
        (
            kept.map(|write| (write.sheet, write.text)),
            fresh.map(|write| (write.sheet, write.text)),
        ),
        (
            Some((Some("s1".to_owned()), ".ad {}\n#cmp {}".to_owned())),
            Some((None, "#cmp {}".to_owned())),
        )
    );
}

/// A click in a frame empties every other document's sheet that holds a
/// rule; a rule set meanwhile waits, the clicked frame's own goes in; and
/// the click's end gives each document its whole text, the waiting rule
/// included. A click in the page's own frame lifts nothing.
#[test]
fn a_click_in_a_frame_lifts_every_other_document_until_it_ends() {
    let mut documents = Documents::default();
    let (main, inner, child) = (
        key("page", "main"),
        key("page", "inner"),
        key("child", "child-main"),
    );
    set(&mut documents, &main, Part::Consent, "#a {}", "s1");
    set(&mut documents, &child, Part::Consent, "#b {}", "s2");
    assert_eq!(documents.lift(&main, &main), Vec::new());
    let lifted = documents.lift(&child, &main);
    for write in &lifted {
        documents.written(write, write.sheet.clone().unwrap_or_default());
    }
    let waiting = documents.set(&main, Part::Hiding, ".ad {}".to_owned());
    let own = documents.set(&child, Part::Consent, "#c {}".to_owned());
    if let Some(write) = &own {
        documents.written(write, "s2".to_owned());
    }
    let quiet = documents.set(&inner, Part::Consent, String::new());
    let restored = documents.restore();
    assert_eq!(
        (
            lifted,
            waiting,
            own.map(|write| write.text),
            quiet,
            restored,
        ),
        (
            vec![Write {
                key: main.clone(),
                enable: false,
                sheet: Some("s1".to_owned()),
                text: String::new(),
            }],
            None,
            Some("#c {}".to_owned()),
            None,
            vec![Write {
                key: main,
                enable: false,
                sheet: Some("s1".to_owned()),
                text: ".ad {}\n#a {}".to_owned(),
            }],
        )
    );
}

/// A write that failed makes the sheet again on the next one, and a
/// detached target or frame takes its documents along.
#[test]
fn a_failed_write_makes_the_sheet_again_and_detached_documents_go() {
    let mut documents = Documents::default();
    let (main, child) = (key("page", "main"), key("child", "child-main"));
    let write = set(&mut documents, &main, Part::Hiding, ".ad {}", "s1");
    documents.failed(&write);
    let again = documents.set(&main, Part::Hiding, ".ad {}".to_owned());
    set(&mut documents, &child, Part::Consent, "#b {}", "s2");
    documents.forget_session(&child.0);
    let after_forget = documents.set(&child, Part::Consent, "#b {}".to_owned());
    documents.forget_frame(&main.0, &main.1);
    let after_frame = documents.set(&main, Part::Hiding, ".ad {}".to_owned());
    assert_eq!(
        (
            again.map(|write| write.sheet),
            after_forget.map(|write| (write.enable, write.sheet)),
            after_frame.map(|write| (write.enable, write.sheet)),
        ),
        (Some(None), Some((true, None)), Some((false, None)))
    );
}

/// In a document a navigation named, live hiding's part waits until the
/// Consent prompt runner's first pass there ends, while the runner's own
/// part goes in; the release writes the whole text once. A release named by
/// another load leaves the document waiting, and one by its own load lets
/// it go.
#[test]
fn live_hiding_waits_in_a_new_document_until_the_runners_first_pass_ends() {
    let mut documents = Documents::default();
    let main = key("page", "main");
    documents.navigated(&main.0, &main.1, "loader-1");

    let waiting = documents.set(&main, Part::Hiding, ".ad {}".to_owned());
    let consent = set(&mut documents, &main, Part::Consent, "#cmp {}", "s1");
    let released = documents.release(&main, None);
    let again = documents.release(&main, None);
    documents.navigated(&main.0, &main.1, "loader-2");
    documents.set(&main, Part::Hiding, ".ad {}".to_owned());
    let other_load = documents.release(&main, Some("loader-1"));
    let own_load = documents.release(&main, Some("loader-2"));

    assert_eq!(
        (
            waiting,
            consent.text,
            released.map(|write| write.text),
            again,
            other_load,
            own_load.map(|write| write.text),
        ),
        (
            None,
            "#cmp {}".to_owned(),
            Some(".ad {}\n#cmp {}".to_owned()),
            None,
            None,
            Some(".ad {}".to_owned()),
        )
    );
}

/// When the guard finds an action's target hidden by the Consent prompt
/// runner, that rule is lifted from the document's sheet (ADR 0048 hides
/// as ADR 0047 does): the runner's later texts keep it out, extraction
/// still removes what the runner hid, and the document's next load holds
/// it again.
#[test]
fn a_rule_of_the_runner_the_guard_lifts_stays_out_of_its_documents_sheet() {
    let mut documents = Documents::default();
    let main = key("page", "main");
    documents.navigated(&main.0, &main.1, "loader-1");
    let runner = "#cmp { display: none !important }\n.cmp-wall { opacity: 0 !important }";
    set(&mut documents, &main, Part::Consent, runner, "s1");

    let lifted = documents.lift_consent(&main, &["#cmp".to_owned()]);
    let lifted_text = lifted.as_ref().map(|write| write.text.clone());
    if let Some(write) = &lifted {
        documents.written(write, "s1".to_owned());
    }
    let rewritten = documents.set(&main, Part::Consent, runner.to_owned());
    let extracted = documents.consent_selectors(&main);
    documents.navigated(&main.0, &main.1, "loader-2");
    let next_load = documents.set(&main, Part::Consent, runner.to_owned());

    assert_eq!(
        (
            lifted_text,
            rewritten,
            extracted,
            next_load.map(|write| write.text)
        ),
        (
            Some(".cmp-wall { opacity: 0 !important }".to_owned()),
            None,
            vec!["#cmp".to_owned(), ".cmp-wall".to_owned()],
            Some(runner.to_owned()),
        )
    );
}
