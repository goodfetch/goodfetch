//! License expressions judged against the repository's own allowlist, as
//! named cases of SPDX's operators and of ADR 0018's rules for each use.

use std::path::Path;

use rstest::rstest;

use super::{Allowlist, Refusal, Verdict};
use crate::notices::entry::Usage;

fn allowlist() -> Allowlist {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("invariant: xtask sits in the repository's root");
    Allowlist::read(root).unwrap()
}

fn allowed(license: &str) -> Verdict {
    Verdict::Allowed(license.to_owned())
}

#[rstest]
#[case::exception(
    "Apache-2.0 WITH LLVM-exception",
    allowed("Apache-2.0 WITH LLVM-exception")
)]
#[case::parentheses(
    "(MIT OR Apache-2.0) AND Unicode-3.0",
    allowed("(MIT OR Apache-2.0) AND Unicode-3.0")
)]
#[case::cargo_legacy_slash("MIT/Apache-2.0", allowed("MIT/Apache-2.0"))]
#[case::lowercase_operators("mit or apache-2.0", allowed("mit or apache-2.0"))]
#[case::gpl_option_left_aside("GPL-2.0-only OR MIT", allowed("GPL-2.0-only OR MIT"))]
#[case::gpl_part_required("MIT AND GPL-2.0-only", Verdict::Refused(Refusal::GplEmbedded))]
#[case::lgpl("LGPL-2.1-or-later", Verdict::Refused(Refusal::GplEmbedded))]
#[case::agpl("AGPL-3.0-only", Verdict::Refused(Refusal::GplEmbedded))]
#[case::other_license("SSPL-1.0", Verdict::Refused(Refusal::NotAllowed))]
#[case::unclosed_parenthesis("(MIT OR Apache-2.0", Verdict::Refused(Refusal::Unknown))]
#[case::exception_missing("Apache-2.0 WITH", Verdict::Refused(Refusal::Unknown))]
#[case::operator_alone("OR", Verdict::Refused(Refusal::Unknown))]
#[case::empty("", Verdict::Refused(Refusal::Unknown))]
fn shipped_unmodified(#[case] stated: &str, #[case] verdict: Verdict) {
    assert_eq!(allowlist().judge(Some(stated), Usage::Unmodified), verdict);
}

#[rstest]
#[case::cc_by_sa_or_gpl("GPL-3.0-or-later OR CC-BY-SA-3.0", allowed("CC-BY-SA-4.0"))]
#[case::cc_by("CC-BY-4.0", allowed("CC-BY-SA-4.0"))]
#[case::mpl("MPL-2.0", allowed("MPL-2.0"))]
#[case::gpl_only("GPL-3.0-only", Verdict::Refused(Refusal::GplEmbedded))]
#[case::non_commercial("CC-BY-NC-SA-4.0", Verdict::Refused(Refusal::NotAllowed))]
fn compiled_filter_list(#[case] stated: &str, #[case] verdict: Verdict) {
    assert_eq!(
        allowlist().judge(Some(stated), Usage::CompiledFilterList),
        verdict
    );
}

#[rstest]
#[case::gpl("GPL-3.0-only", allowed("GPL-3.0-only"))]
#[case::cc_by_sa("CC-BY-SA-4.0", allowed("CC-BY-SA-4.0"))]
#[case::non_commercial("CC-BY-NC-SA-4.0", Verdict::Refused(Refusal::NotAllowed))]
fn download_only(#[case] stated: &str, #[case] verdict: Verdict) {
    assert_eq!(
        allowlist().judge(Some(stated), Usage::DownloadOnly),
        verdict
    );
}

#[rstest]
#[case::mpl("MPL-2.0", Verdict::Refused(Refusal::Modified))]
#[case::permissive("BSD-3-Clause", allowed("BSD-3-Clause"))]
fn modified(#[case] stated: &str, #[case] verdict: Verdict) {
    assert_eq!(allowlist().judge(Some(stated), Usage::Modified), verdict);
}

#[test]
fn nothing_stated_is_unknown() {
    assert_eq!(
        allowlist().judge(None, Usage::Unmodified),
        Verdict::Refused(Refusal::Unknown)
    );
}
