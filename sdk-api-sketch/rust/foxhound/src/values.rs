//! Foxhound's values of the open value types both Products share (ADR 0099): free constants, each
//! equal with `==` and in a `match` to a value read from the wire, as Foxhound's Error codes are in
//! [`crate::error_code`].

/// Foxhound's Limit scopes.
pub mod limit_scope {
    use goodfetch_collie_client::LimitScope;

    /// A Partitioned crawl (ADR 0054).
    pub const PARTITIONED_CRAWL: LimitScope = LimitScope::new("partitioned_crawl");
}

/// Foxhound's kinds of Usage.
pub mod usage_kind {
    use goodfetch_collie_client::UsageKind;

    /// Paid fetches, counted (ADR 0045).
    pub const PAID_FETCHES: UsageKind = UsageKind::new("paid_fetches");
}

/// Foxhound's Hook stages (ADRs 0088, 0104).
pub mod hook_stage {
    use goodfetch_collie_client::HookStage;

    /// Before each fetch of a request: also the stage before a retry.
    pub const BEFORE_REQUEST: HookStage = HookStage::new("before_request");
    /// After the final response, before the status rule.
    pub const AFTER_RESPONSE: HookStage = HookStage::new("after_response");
    /// Before each Record's commit.
    pub const BEFORE_RECORD_COMMIT: HookStage = HookStage::new("before_record_commit");
    /// Observe-only, for process Plugins and Script Hooks: a request's outcome.
    pub const REQUEST_ENDED: HookStage = HookStage::new("request_ended");
}
