//! The fixture's stand-in of test support: one type whose schema
//! `goodfetch-test-env --print-schemas` prints, and the Secret canary kinds
//! whose `gitleaks` rule `--print-canary-rule` prints.

/// One test's setup.
#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct Description {
    /// The test's seed.
    pub seed: u64,
}

/// The Secret canary kinds, each with its value's pattern.
pub const KINDS: [(&str, &str); 1] = [("typed-password", "gfcanary-password-[0-9a-f]{16}")];

/// The `gitleaks` rule: one rule per kind.
pub fn gitleaks_rule() -> String {
    KINDS
        .iter()
        .map(|(kind, pattern)| format!("[[rules]]\nid = \"{kind}\"\nregex = '''{pattern}'''\n"))
        .collect()
}
