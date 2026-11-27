//! Lead Daemon confirmation and fencing (ADR 0055): a Daemon configured as
//! Lead Daemon acts as the single writer of the group's shared control state
//! only after it has confirmed its lead epoch with a majority of its
//! last-known Group members (itself included), or an operator has confirmed
//! it with `group lead --confirm`.
//!
//! If any Group member answers with a higher lead epoch, this Lead Daemon
//! is fenced off and refuses both acting and hand confirmation with
//! `lead_epoch_stale`. Every state transition is emitted under
//! [`crate::group::TARGET`] so TLC validates Simulation traces against
//! `test-support/models/lead-epoch/LeadEpoch.tla` (ADR 0060).

use std::collections::BTreeSet;

use goodfetch_protocol::group::{LeadEpoch, LeadEpochStaleDetails};

use super::TARGET;

/// The warning ADR 0055 requires whenever an operator confirms a Lead Daemon
/// by hand without reaching a majority of its last-known Group members.
pub(crate) const OPERATOR_CONFIRM_WARNING: &str = "Confirmed without reaching a majority of last-known Group members: the former Lead Daemon must not restart with its lead configuration.";

/// Why a Lead Daemon cannot perform a single-writer action right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeadRefusal {
    /// This Daemon is not the Lead Daemon.
    NotLead,
    /// It has not yet confirmed its lead epoch with a majority or
    /// `group lead --confirm`.
    Unconfirmed {
        /// The epoch waiting for confirmation.
        lead_epoch: LeadEpoch,
        /// How many last-known Group members (itself included) confirmed it.
        confirmed_by: usize,
        /// How many last-known Group members the group had.
        total_members: usize,
    },
    /// A Group member reported a higher lead epoch, fencing this Lead Daemon.
    Fenced(LeadEpochStaleDetails),
}

/// What one peer exchange with a last-known Group member produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExchangeOutcome {
    /// Still waiting for more Group members to reach a strict majority.
    Pending,
    /// This exchange completed a strict majority, confirming the Lead Daemon.
    Confirmed,
    /// The Group member reported a higher lead epoch, fencing this Lead Daemon.
    Fenced(LeadEpoch),
    /// The Lead Daemon was already confirmed, fenced, or not a Lead Daemon.
    Unchanged,
}

/// Snapshot of a Lead Daemon's confirmation state for `group.lead`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeadSnapshot {
    /// The epoch this Lead Daemon is configured to lead at.
    pub lead_epoch: LeadEpoch,
    /// Whether it has confirmed and may act.
    pub confirmed: bool,
    /// Whether confirmation came from `group lead --confirm`.
    pub confirmed_by_operator: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    NotLead,
    Unconfirmed {
        epoch: LeadEpoch,
        members: BTreeSet<String>,
        confirmed_by: BTreeSet<String>,
    },
    Confirmed {
        epoch: LeadEpoch,
        by_operator: bool,
        members: BTreeSet<String>,
    },
    Fenced {
        epoch: LeadEpoch,
        higher: LeadEpoch,
    },
}

/// A Daemon's Lead Daemon confirmation state machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leadership {
    self_name: String,
    state: State,
}

impl Leadership {
    /// A Group member that is not configured as Lead Daemon.
    pub fn not_lead(self_name: impl Into<String>) -> Self {
        Self {
            self_name: self_name.into(),
            state: State::NotLead,
        }
    }

    /// A Daemon starting as Lead Daemon `self_name` at `epoch` with
    /// `last_known_members` (which always includes `self_name`).
    ///
    /// When `self_name` alone is already a strict majority (a 1-member group,
    /// as right after `group init`), it starts confirmed immediately.
    pub fn starting(
        self_name: impl Into<String>,
        epoch: LeadEpoch,
        last_known_members: impl IntoIterator<Item = String>,
    ) -> Self {
        let self_name = self_name.into();
        let mut members: BTreeSet<String> = last_known_members.into_iter().collect();
        members.insert(self_name.clone());
        let mut confirmed_by = BTreeSet::new();
        confirmed_by.insert(self_name.clone());
        let state = if is_majority(confirmed_by.len(), members.len()) {
            tracing::info!(
                target: TARGET,
                transition = "confirm_majority",
                member = %self_name,
                lead_epoch = epoch.get(),
                "the Lead Daemon confirmed its lead epoch as the sole Group member"
            );
            State::Confirmed {
                epoch,
                by_operator: false,
                members,
            }
        } else {
            State::Unconfirmed {
                epoch,
                members,
                confirmed_by,
            }
        };
        Self { self_name, state }
    }

    /// Whether this Daemon is a Lead Daemon still waiting for majority
    /// confirmation.
    pub fn is_unconfirmed(&self) -> bool {
        matches!(self.state, State::Unconfirmed { .. })
    }

    /// The last-known Group members other than this Lead Daemon that have not
    /// yet confirmed the current lead epoch.
    pub fn unconfirmed_peers(&self) -> Vec<String> {
        match &self.state {
            State::Unconfirmed {
                members,
                confirmed_by,
                ..
            } => members
                .iter()
                .filter(|member| !confirmed_by.contains(*member))
                .cloned()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Records the lead epoch `peer_epoch` that the Group member `peer`
    /// answered with during confirmation (or on any peer link).
    pub fn record_peer_exchange(&mut self, peer: &str, peer_epoch: LeadEpoch) -> ExchangeOutcome {
        match &mut self.state {
            State::Unconfirmed {
                epoch,
                members,
                confirmed_by,
            } => {
                let current = *epoch;
                if peer_epoch > current {
                    tracing::warn!(
                        target: TARGET,
                        transition = "fence",
                        member = %self.self_name,
                        lead_epoch = current.get(),
                        highest_lead_epoch = peer_epoch.get(),
                        "a Group member reported a higher lead epoch, fencing this Lead Daemon"
                    );
                    self.state = State::Fenced {
                        epoch: current,
                        higher: peer_epoch,
                    };
                    return ExchangeOutcome::Fenced(peer_epoch);
                }
                if peer_epoch == current && members.contains(peer) {
                    if confirmed_by.insert(peer.to_owned()) {
                        tracing::debug!(
                            target: TARGET,
                            transition = "ack",
                            member = %self.self_name,
                            peer = %peer,
                            lead_epoch = current.get(),
                            "a Group member acknowledged the lead epoch"
                        );
                    }
                    if is_majority(confirmed_by.len(), members.len()) {
                        tracing::info!(
                            target: TARGET,
                            transition = "confirm_majority",
                            member = %self.self_name,
                            lead_epoch = current.get(),
                            "the Lead Daemon confirmed its lead epoch with a majority of its last-known Group members"
                        );
                        let members = std::mem::take(members);
                        self.state = State::Confirmed {
                            epoch: current,
                            by_operator: false,
                            members,
                        };
                        return ExchangeOutcome::Confirmed;
                    }
                }
                ExchangeOutcome::Pending
            }
            State::Confirmed { epoch, .. } if peer_epoch > *epoch => {
                let current = *epoch;
                tracing::warn!(
                    target: TARGET,
                    transition = "fence",
                    member = %self.self_name,
                    lead_epoch = current.get(),
                    highest_lead_epoch = peer_epoch.get(),
                    "a higher lead epoch fenced this Lead Daemon"
                );
                self.state = State::Fenced {
                    epoch: current,
                    higher: peer_epoch,
                };
                ExchangeOutcome::Fenced(peer_epoch)
            }
            State::Fenced { higher, .. } if peer_epoch > *higher => {
                *higher = peer_epoch;
                ExchangeOutcome::Fenced(peer_epoch)
            }
            _ => ExchangeOutcome::Unchanged,
        }
    }

    /// Fences this Lead Daemon when a peer message or local epoch raise shows
    /// `higher` above its lead epoch.
    pub fn fence_if_higher(&mut self, higher: LeadEpoch) {
        let current = match self.state {
            State::Unconfirmed { epoch, .. } | State::Confirmed { epoch, .. } => epoch,
            State::Fenced {
                higher: ref mut kept,
                ..
            } => {
                if higher > *kept {
                    *kept = higher;
                }
                return;
            }
            State::NotLead => return,
        };
        if higher > current {
            tracing::warn!(
                target: TARGET,
                transition = "fence",
                member = %self.self_name,
                lead_epoch = current.get(),
                highest_lead_epoch = higher.get(),
                "a higher lead epoch fenced this Lead Daemon"
            );
            self.state = State::Fenced {
                epoch: current,
                higher,
            };
        }
    }

    /// Confirms this Lead Daemon by hand (`group lead --confirm`), unless it
    /// is not a Lead Daemon or has already been fenced by a higher lead epoch.
    pub fn confirm_by_operator(&mut self) -> Result<LeadSnapshot, LeadRefusal> {
        match &mut self.state {
            State::NotLead => Err(LeadRefusal::NotLead),
            State::Fenced { epoch, higher } => Err(LeadRefusal::Fenced(LeadEpochStaleDetails {
                lead_epoch: *epoch,
                highest_lead_epoch: *higher,
            })),
            State::Confirmed {
                epoch, by_operator, ..
            } => Ok(LeadSnapshot {
                lead_epoch: *epoch,
                confirmed: true,
                confirmed_by_operator: *by_operator,
            }),
            State::Unconfirmed { epoch, members, .. } => {
                let epoch = *epoch;
                let members = std::mem::take(members);
                tracing::warn!(
                    target: TARGET,
                    transition = "confirm_operator",
                    member = %self.self_name,
                    lead_epoch = epoch.get(),
                    "an operator confirmed this Lead Daemon without reaching a majority; the former Lead Daemon must not restart with its lead configuration"
                );
                self.state = State::Confirmed {
                    epoch,
                    by_operator: true,
                    members,
                };
                Ok(LeadSnapshot {
                    lead_epoch: epoch,
                    confirmed: true,
                    confirmed_by_operator: true,
                })
            }
        }
    }

    /// Checks whether this Lead Daemon may perform a single-writer action right
    /// now, returning its confirmed lead epoch or the reason it cannot.
    pub fn can_act(&self) -> Result<LeadEpoch, LeadRefusal> {
        match &self.state {
            State::NotLead => Err(LeadRefusal::NotLead),
            State::Unconfirmed {
                epoch,
                members,
                confirmed_by,
            } => Err(LeadRefusal::Unconfirmed {
                lead_epoch: *epoch,
                confirmed_by: confirmed_by.len(),
                total_members: members.len(),
            }),
            State::Confirmed { epoch, .. } => Ok(*epoch),
            State::Fenced { epoch, higher } => Err(LeadRefusal::Fenced(LeadEpochStaleDetails {
                lead_epoch: *epoch,
                highest_lead_epoch: *higher,
            })),
        }
    }

    /// Current status of this Lead Daemon for `group.lead` (without confirming).
    pub fn status(&self) -> Result<LeadSnapshot, LeadRefusal> {
        match &self.state {
            State::NotLead => Err(LeadRefusal::NotLead),
            State::Fenced { epoch, higher } => Err(LeadRefusal::Fenced(LeadEpochStaleDetails {
                lead_epoch: *epoch,
                highest_lead_epoch: *higher,
            })),
            State::Unconfirmed { epoch, .. } => Ok(LeadSnapshot {
                lead_epoch: *epoch,
                confirmed: false,
                confirmed_by_operator: false,
            }),
            State::Confirmed {
                epoch, by_operator, ..
            } => Ok(LeadSnapshot {
                lead_epoch: *epoch,
                confirmed: true,
                confirmed_by_operator: *by_operator,
            }),
        }
    }

    /// Records a newly admitted Group member once confirmed (ADR 0055).
    pub fn on_member_added(&mut self, member: String) {
        if let State::Confirmed {
            epoch,
            ref mut members,
            ..
        } = self.state
        {
            tracing::debug!(
                target: TARGET,
                transition = "add_member",
                member = %self.self_name,
                target = %member,
                lead_epoch = epoch.get(),
                "the Lead Daemon admitted a Group member"
            );
            members.insert(member);
        }
    }

    /// Records a removed Group member once confirmed (ADR 0055).
    pub fn on_member_removed(&mut self, member: &str) {
        if let State::Confirmed {
            epoch,
            ref mut members,
            ..
        } = self.state
        {
            tracing::debug!(
                target: TARGET,
                transition = "remove_member",
                member = %self.self_name,
                target = %member,
                lead_epoch = epoch.get(),
                "the Lead Daemon removed a Group member"
            );
            members.remove(member);
        }
    }
}

/// Strict majority of `total` members: strictly more than half.
fn is_majority(reached: usize, total: usize) -> bool {
    reached.saturating_mul(2) > total
}

#[cfg(test)]
mod tests;
