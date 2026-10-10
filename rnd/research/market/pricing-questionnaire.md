# Pricing questionnaire: Van Westendorp and Gabor-Granger

**Status:** v1, ready to use. **Date:** 2026-10-08.
**Purpose:** measure the acceptable price range and the purchase probability for the `verified-result` unit of `D-pricing`, and compare it with the two other options (`compute-units`, `subscription-tiers`). To be asked at the end of the [interview](interview-guide.md), then online to broaden the sample.

## 1. Define the unit before any question

A price question without a defined unit measures nothing. Read word for word one of the two definitions depending on the segment:

- **Reading** (`data-teams`, `ecommerce-pricing`): "A *verified result* is a page read whose data passed an accuracy check. A blocked, incomplete or failed page is never billed. Unit: 1,000 verified pages."
- **Task** (`agent-developers`, `b2b-portals`): "A *verified result* is an agent task carried through to completion, whose effect (form submitted, data retrieved) is checked. A failed or falsely successful task is never billed. Unit: 1,000 verified tasks."

Then specify the context: hosting included, no commitment, monthly spending cap set by the customer.

## 2. Van Westendorp (four questions, free answers in dollars)

Ask the questions in this order, without suggesting values:

1. **Too expensive.** At what price for 1,000 verified results would you consider the offer so expensive that you would not consider it?
2. **Expensive.** At what price would you start to find it expensive, while still considering it?
3. **Cheap.** At what price would you find it a good deal?
4. **Too cheap.** At what price would it be so cheap that you would doubt its quality?

Consistency check: we expect too cheap ≤ cheap ≤ expensive ≤ too expensive. An inconsistent answer is excluded from the analysis and counted separately.

**Analysis.** Plot the cumulative distributions. The two usual intersections are used:
- **lower bound**: too cheap crosses expensive;
- **upper bound**: too expensive crosses cheap.

The acceptable price range lies between the two. The curves are only stable from about 30 responses per segment. Below that, with only the 10 to 15 interviews, report the individual answers and their median, without computing an optimal point.

## 3. Gabor-Granger (purchase probability at imposed prices)

Response scale: 1 *definitely not*, 2 *probably not*, 3 *maybe*, 4 *probably*, 5 *definitely*. Start at a randomly drawn step, then go up if the answer is 4 or 5, down otherwise. Do not show the whole scale.

| Unit | Steps (USD per 1,000 verified results) | Anchor observed in the market lane |
| --- | --- | --- |
| 1,000 verified pages | 0.50 · 1 · 2 · 4 · 8 · 15 | ~$2/1k pages requested (`reddit-81`); $0.005/page advertised (`demand-use-cases-49` `[P]`) |
| 1,000 verified tasks | 5 · 10 · 25 · 50 · 100 · 250 | $0.01-0.05/task with a replayed plan (`reddit-62` `[P]`); ~$0.21/task at Notte (`hn-75` `[P]`) |

The anchors are used to place the scale, not to suggest a price to the respondent.

**Analysis.** For each step, count the share of answers 4 or 5. Relative revenue equals price × share. Report the demand curve and the step that maximizes this relative revenue, per segment, with the sample size.

## 4. Preferred unit and cap

- **Ranking of units.** Rank these three ways of paying, from most preferred to least liked: per verified result, per detailed resources consumed, monthly capped subscription. Why the first one? (`D-pricing`)
- **Verification premium.** If a competitor bills 1,000 pages at price X, failures included, how much more would you accept to pay to pay only for verified results? Free answer, as a percentage.
- **Cap.** What monthly cap would you set from the first month?
- **Last billing incident.** Describe the last invoice that surprised you: amount, cause, what you did.

## 5. Going online and next steps

- Online version: an anonymous form, with no mandatory address, with the interview's screening screen and sections 1 to 4. Target: 30 consistent responses per retained segment.
- Then, a price test on a landing page: two variants (verified result versus capped subscription), measured by the waiting-list sign-up rate ([pricing-signals.md](pricing-signals.md), §5).
- Only a paid PoC constitutes proof of payment. The answers to this questionnaire bound the prices to test; they do not set the rate.
- The results go under `research/market/pricing-results.md` and in the `evidence` of `D-pricing`.
