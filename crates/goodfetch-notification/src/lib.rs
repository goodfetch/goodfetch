//! Notifications (ADRs 0027 and 0129; spec #608): messages about the
//! progress or outcome of a Run or a Schedule, sent to a person or a
//! system, which carry no Records. The Notification channel driver owns
//! what every channel shares: the delivery ledger ([`ledger`]), keyed by
//! event ID so a resumed Run never sends an event again, retries on the
//! delivery schedule, the rate limit of each destination ([`rate_limit`])
//! and the failure enum ([`failure`]) each channel's translation point
//! maps its party's answers to.

pub mod failure;
pub mod ledger;
pub mod message;
pub mod rate_limit;
