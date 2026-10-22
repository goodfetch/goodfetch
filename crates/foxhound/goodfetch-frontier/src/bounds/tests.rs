//! The caps on a URL's shape as a pure function of the URL and the caps
//! (spec #581's Testing Decisions: pure rules in process), against ADR
//! 0120's table and TESTING.md's URL identity cases.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::foxhound::frontier::NotAddedReason;
use proptest::prelude::*;
use rstest::rstest;
use url::Url;

use super::Bounds;

/// `http://caps.test` with `path`.
fn url_of(path: &str) -> Url {
    Url::parse(&format!("http://caps.test{path}")).unwrap()
}

/// A URL whose path is `count` segments, numbered from 1.
fn numbered(count: u32) -> String {
    let path = (1..=count)
        .map(|n| format!("/{n}"))
        .collect::<Vec<_>>()
        .concat();
    format!("http://caps.test{path}")
}

/// A URL of exactly `length` characters as parsed, its fragment aside.
fn url_of_length(length: usize) -> String {
    let start = "http://caps.test/";
    format!("{start}{}#fragment", "a".repeat(length - start.len()))
}

/// The built-in caps refuse what ADR 0120's table says, by the name of the
/// setting past which a URL is, and admit what stays within them.
///
/// Covers: story:6-228, setting:crawl.max_url_length, setting:crawl.max_path_segments, setting:crawl.max_segment_repeats
#[rstest]
#[case::the_longest_url(&url_of_length(2_083), None)]
#[case::one_character_longer(&url_of_length(2_084), Some(NotAddedReason::MaxUrlLength))]
#[case::three_thousand_characters(&url_of_length(3_000), Some(NotAddedReason::MaxUrlLength))]
#[case::twenty_segments(&numbered(20), None)]
#[case::twenty_segments_and_a_trailing_slash(&format!("{}/", numbered(20)), None)]
#[case::twenty_one_segments(&numbered(21), Some(NotAddedReason::MaxPathSegments))]
#[case::twenty_five_segments(&numbered(25), Some(NotAddedReason::MaxPathSegments))]
#[case::a_segment_twice("http://caps.test/a/a/", None)]
#[case::a_segment_three_times("http://caps.test/a/a/a/", Some(NotAddedReason::MaxSegmentRepeats))]
#[case::a_segment_three_times_deeper(
    "http://caps.test/archive/more/more/more/",
    Some(NotAddedReason::MaxSegmentRepeats)
)]
#[case::a_run_of_segments_three_times(
    "http://caps.test/a/b/a/b/a/b/",
    Some(NotAddedReason::MaxSegmentRepeats)
)]
#[case::a_run_of_three_three_times(
    "http://caps.test/x/a/b/c/a/b/c/a/b/c",
    Some(NotAddedReason::MaxSegmentRepeats)
)]
#[case::a_run_twice("http://caps.test/a/b/a/b/c", None)]
#[case::numbers_apart("http://caps.test/page/1/tag/1/sort/1", None)]
#[case::the_same_segment_apart("http://caps.test/a/b/a/c/a", None)]
#[case::the_query_aside("http://caps.test/a?x=/a/a/a", None)]
fn the_built_in_caps_refuse_a_url_past_one(
    #[case] url: &str,
    #[case] refused: Option<NotAddedReason>,
) {
    let url = Url::parse(url).unwrap();

    assert_eq!(Bounds::default().shape_refuses(&url), refused);
}

/// Each cap set to `null` admits the URL it refused, and a cap set lower
/// refuses what the built-in one admits.
///
/// Covers: story:6-228, setting:crawl.max_url_length, setting:crawl.max_path_segments, setting:crawl.max_segment_repeats
#[test]
fn a_cap_set_to_null_admits_its_url_and_one_set_lower_refuses_more() {
    let long = Url::parse(&url_of_length(3_000)).unwrap();
    let deep = Url::parse(&numbered(25)).unwrap();
    let looping = url_of("/a/a/a/");
    let off = Bounds {
        max_url_length: None,
        max_path_segments: None,
        max_segment_repeats: None,
        ..Bounds::default()
    };
    let only = |caps: [Option<u64>; 3]| Bounds {
        max_url_length: caps[0],
        max_path_segments: caps[1],
        max_segment_repeats: caps[2],
        ..Bounds::default()
    };

    assert_eq!(
        (
            [&long, &deep, &looping].map(|url| off.shape_refuses(url)),
            [
                (only([Some(20), None, None]), "/a/bc"),
                (only([Some(20), None, None]), "/a/b"),
                (only([None, Some(1), None]), "/a/b"),
                (only([None, None, Some(1)]), "/a/a"),
                (only([None, None, Some(1)]), "/a/b/a"),
                (only([None, None, Some(0)]), "/a"),
            ]
            .map(|(bounds, path)| bounds.shape_refuses(&url_of(path))),
        ),
        (
            [None, None, None],
            [
                Some(NotAddedReason::MaxUrlLength),
                None,
                Some(NotAddedReason::MaxPathSegments),
                Some(NotAddedReason::MaxSegmentRepeats),
                None,
                Some(NotAddedReason::MaxSegmentRepeats),
            ]
        )
    );
}

/// Whether a run of `segments` appears more than `max` times back to back,
/// by brute force: every start, every width, every count of blocks.
fn repeats_by_brute_force(segments: &[&str], max: usize) -> bool {
    (0..segments.len()).any(|start| {
        (1..=segments.len() - start).any(|width| {
            let block = &segments[start..start + width];
            let blocks = segments[start..]
                .chunks_exact(width)
                .take_while(|chunk| *chunk == block)
                .count();
            blocks > max
        })
    })
}

proptest! {
    /// `crawl.max_segment_repeats` refuses exactly the paths in which a
    /// segment, or a run of them, appears more than the cap back to back.
    ///
    /// Covers: story:6-228, setting:crawl.max_segment_repeats
    #[test]
    fn the_repeat_cap_refuses_a_run_past_it_and_nothing_else(
        segments in prop::collection::vec(prop::sample::select(vec!["a", "b", "c"]), 0..12),
        max in 0_u64..4,
    ) {
        let path = segments.iter().map(|segment| format!("/{segment}")).collect::<Vec<_>>().concat();
        let bounds = Bounds {
            max_url_length: None,
            max_path_segments: None,
            max_segment_repeats: Some(max),
            ..Bounds::default()
        };

        prop_assert_eq!(
            bounds.shape_refuses(&url_of(&path)) == Some(NotAddedReason::MaxSegmentRepeats),
            repeats_by_brute_force(&segments, usize::try_from(max).unwrap())
        );
    }
}
