use goodfetch_cdp::SessionId as TargetSession;

use super::{Frame, Frames};

fn session(name: &str) -> TargetSession {
    TargetSession(name.to_owned())
}

fn frame(session_name: &str, frame: &str) -> Frame {
    Frame {
        session: session(session_name),
        id: frame.to_owned(),
        context: 1,
    }
}

/// The page with an in-process iframe `inner`, an out-of-process iframe
/// target `child` (frame `child-main`) holding an in-process iframe
/// `child-inner`.
fn page() -> Frames {
    let mut frames = Frames::new(session("page"), "main".to_owned());
    frames.attach(&session("page"), &session("child"), "child-main".to_owned());
    frames
}

/// A click's point is read through each owner between its frame and the
/// page: none for the page's own frame; an in-process iframe's owner in
/// its target; an out-of-process iframe's owner in its parent target; and
/// for a frame in an out-of-process iframe, its owner there, then that
/// target's owner in the page.
#[test]
fn the_way_up_from_a_frame_names_each_owner_with_the_target_that_holds_it() {
    let frames = page();
    assert_eq!(
        [
            frames.owner_chain(&frame("page", "main")),
            frames.owner_chain(&frame("page", "inner")),
            frames.owner_chain(&frame("child", "child-main")),
            frames.owner_chain(&frame("child", "child-inner")),
        ],
        [
            Some(vec![]),
            Some(vec![(session("page"), "inner".to_owned())]),
            Some(vec![(session("page"), "child-main".to_owned())]),
            Some(vec![
                (session("child"), "child-inner".to_owned()),
                (session("page"), "child-main".to_owned()),
            ]),
        ]
    );
}

/// A frame of a target that detached has no way up.
#[test]
fn a_frame_of_a_detached_target_has_no_way_up() {
    let mut frames = page();
    frames.forget_session(&session("child"));
    assert_eq!(frames.owner_chain(&frame("child", "child-main")), None);
}
