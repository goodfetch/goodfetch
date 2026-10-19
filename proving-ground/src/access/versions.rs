//! Chrome Stable's majors with the date each became Stable, written by hand
//! from Chromium's published release schedule, and the version check's
//! grades against them (ADR 0035): a build 1 major behind Stable is `info`,
//! 2 or 3 behind a `notice`, 4 or more behind a `warning`. The table and the
//! date it is read at move together, by hand, so a grade never depends on
//! the wall clock.

use serde::Serialize;

/// The date the version check is graded at.
pub(crate) const AS_OF: &str = "2026-10-19";

/// Each Chrome major and the date it became Stable, from Chromium's release
/// schedule (`chromiumdash.appspot.com`, read on 2026-10-19).
const STABLE: [(u32, &str); 24] = [
    (136, "2025-04-29"),
    (137, "2025-05-27"),
    (138, "2025-06-24"),
    (139, "2025-08-05"),
    (140, "2025-09-02"),
    (141, "2025-09-30"),
    (142, "2025-10-28"),
    (143, "2025-12-02"),
    (144, "2026-01-13"),
    (145, "2026-02-10"),
    (146, "2026-03-10"),
    (147, "2026-04-07"),
    (148, "2026-05-05"),
    (149, "2026-06-02"),
    (150, "2026-06-30"),
    (151, "2026-07-28"),
    (152, "2026-08-25"),
    (153, "2026-09-08"),
    (154, "2026-09-22"),
    (155, "2026-10-06"),
    (156, "2026-10-20"),
    (157, "2026-11-03"),
    (158, "2026-11-17"),
    (159, "2026-12-08"),
];

/// A version check's grade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Level {
    /// Stable's major, or a newer one.
    Current,
    Info,
    Notice,
    /// Outdated.
    Warning,
}

impl Level {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Info => "info",
            Self::Notice => "notice",
            Self::Warning => "warning",
        }
    }
}

/// How far a build is behind Chrome Stable on [`AS_OF`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct BuildAge {
    pub(crate) stable_major: u32,
    pub(crate) versions_behind: u32,
    pub(crate) level: Level,
}

/// The age of a build of Chrome major `major` on [`AS_OF`].
pub(crate) fn age(major: u32) -> BuildAge {
    let stable_major = STABLE
        .iter()
        .filter(|(_, date)| *date <= AS_OF)
        .map(|(major, _)| *major)
        .max()
        .expect("invariant: the table holds a major Stable on its own date");
    let versions_behind = stable_major.saturating_sub(major);
    let level = match versions_behind {
        0 => Level::Current,
        1 => Level::Info,
        2 | 3 => Level::Notice,
        _ => Level::Warning,
    };
    BuildAge {
        stable_major,
        versions_behind,
        level,
    }
}

/// The Chrome major a user agent claims (`Chrome/155.0.0.0`, headless
/// Chrome's `HeadlessChrome/155.0.0.0` included), if it claims one.
pub(crate) fn claimed_major(user_agent: &str) -> Option<u32> {
    let (_, rest) = user_agent.split_once("Chrome/")?;
    rest.split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// The Chrome major the `Sec-CH-UA` client hint claims through its
/// `Chromium`, `Google Chrome` or `HeadlessChrome` brand, if any.
pub(crate) fn hinted_major(brands: &str) -> Option<u32> {
    brands.split(',').find_map(|brand| {
        let (name, version) = brand.trim().split_once(";v=")?;
        matches!(
            name.trim_matches('"'),
            "Chromium" | "Google Chrome" | "HeadlessChrome"
        )
        .then(|| version.trim_matches('"').parse().ok())
        .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_agent_names_its_chrome_major() {
        assert_eq!(
            claimed_major(
                "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.7390.54 Safari/537.36"
            ),
            Some(155)
        );
        assert_eq!(
            claimed_major("Mozilla/5.0 HeadlessChrome/131.0.0.0"),
            Some(131)
        );
        assert_eq!(claimed_major("curl/8.7.1"), None);
    }

    #[test]
    fn client_hints_name_their_chrome_major() {
        assert_eq!(
            hinted_major(r#""Not.A/Brand";v="99", "Chromium";v="155", "Google Chrome";v="155""#),
            Some(155)
        );
        assert_eq!(hinted_major(r#""Not.A/Brand";v="99""#), None);
    }

    #[test]
    fn a_major_older_than_the_table_is_outdated() {
        assert_eq!(
            age(120),
            BuildAge {
                stable_major: 155,
                versions_behind: 35,
                level: Level::Warning
            }
        );
    }
}
