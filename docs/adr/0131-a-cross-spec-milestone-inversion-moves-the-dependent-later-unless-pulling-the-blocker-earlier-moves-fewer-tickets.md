# A cross-spec milestone inversion moves the dependent later, unless pulling the blocker earlier moves fewer tickets

When a ticket from one spec is blocked by a ticket from another spec that sits in a later milestone, `/to-tickets` moves the dependent later, or splits off the part that needs the later capability, so each capability keeps the milestone ADR 0058 gives it. It pulls the blocker earlier instead only when that changes the milestone of fewer tickets. Within one spec, a blocker later than what it blocks still moves earlier, as ADR 0058 says.

ADR 0058 made `/to-tickets` move a blocker earlier whenever it was later than what it blocks. That rule was written for one spec's tickets. Applied across specs, it pulls whole capabilities into an earlier milestone because one ticket of another area mentions them. When the tickets of V1 areas 4 to 14 were linked across areas, one security ticket of area 4 (M1) that names Borrowed browsers would have pulled area 5's Borrowed browsers, Firefox, WebKit and Camoufox, all M3 under ADR 0058, into M1. The cross-area review found 16 inversions on 12 tickets; moving the dependent fixed most of them with no other move, and pulling a small blocker earlier fixed two with fewer moves than the rule's alternative.

Decided with the owner while reviewing the V1 tickets' cross-area edges on 2026-10-16 and 2026-10-17 (the area spec session's Rounds 19 and 23).

## Decision

- **Within one spec:** a blocker later than what it blocks moves earlier (ADR 0058, unchanged).
- **Across specs, the default:** the dependent moves to the blocker's milestone, or is split: the part that needs the later capability becomes its own ticket in the blocker's milestone, blocked by it, and the rest keeps its milestone.
- **The exception:** the blocker moves earlier instead when that changes the milestone of fewer tickets. The count on each side is the tickets whose milestone must change, the moved ticket included, following blocking edges transitively (dependents for a move later, blockers for a move earlier). On a tie, the dependent moves.
- **A dependency that is only an extension is no edge.** When a ticket works without the other spec's capability and only extends to it once that lands, the provider gets a "When this lands" note in a comment, and no blocking edge.
- **Every cross-spec move is listed for the owner** in the `/to-tickets` quiz that finds it, and each moved ticket gets a comment saying why.

> Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): `/to-tickets` applies no milestone step, and `docs/agents/issue-tracker.md` keeps only the invariant that a ticket's milestone is never later than any ticket it blocks. This ADR records how the tickets of areas 4 to 14 were moved.

> Amended by [ADR 0136](0136-each-implement-spec-run-is-one-whole-spec-and-milestones-are-ticket-metadata-and-owner-checkpoints.md): no owner test holds back a ticket, so a milestone inversion no longer stalls work. The invariant still holds, as ticket metadata.

## Considered Options

- **ADR 0058's rule everywhere:** rejected. Across specs it moves whole capabilities into an earlier milestone, against ADR 0058's own plan of what each milestone delivers.
- **Always move the dependent:** rejected. Moving A11-F10 (Foxhound's crawl context on the document tiers) to M3 would have moved 13 tickets, golden scenario 12's Journey among them, where pulling one locator ticket to M2 moved two.

## Consequences

- **Amends ADR 0058.** Its rule that `/to-tickets` moves a blocker earlier holds within one spec; across specs, this ADR's rule applies. A note sits at that bullet.
- **`docs/agents/issue-tracker.md`'s Order rule** states both cases and cites this ADR.
- **Applied to the tickets of areas 4 to 14:** A08-08, A08-21, A08-37, A08-41 and A04-74 moved to M4; A11-058 and A05-27 to M2; A11-015 to M1. Each carries a comment.
- **ADR 0037.** No area builds this ADR's decision; it governs `/to-tickets`, so no area row lists it.
- **Glossary.** No term changes.
