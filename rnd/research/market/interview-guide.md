# "Switch" interview guide (Jobs To Be Done)

**Status:** v1, ready to use. **Date:** 2026-10-08.
**Purpose:** settle `D-segment`, `D-feature-priority` and the qualitative part of `D-pricing` based on real purchase decisions, not opinions. The pricing questionnaire is separate: [pricing-questionnaire.md](pricing-questionnaire.md).
**Who leads:** the project owner. I code the transcripts with the [codebook](codebook.md).

A "switch" interview reconstructs the last time the person **changed** tools (or decided to build their own): what pushed them, what attracted them, what held them back. Past facts predict purchases better than intentions. The golden rule: ask what happened, never what the person would do.

## 1. Recruitment

**Quota.** 10 to 15 interviews, at least 3 per candidate segment of `D-segment`, plus 2 people who **left** a hosted tool for a homegrown one.

| Segment (`D-segment`) | Profile to recruit | Eligibility criterion |
| --- | --- | --- |
| `agent-developers` | Develops an agent that reads or acts on the web | Changed browser tool, agent framework or access provider in the last 12 months |
| `b2b-portals` | Automates authenticated portals (insurance, logistics, government, ERP) | At least one portal in production; has experienced a breakage or a session expiry |
| `data-teams` | Team that collects web data at volume | Existing proxy, captcha or access API budget |
| `ecommerce-pricing` | Price or catalog monitoring | Tracks at least 100 competitor products |

**Where to find them.** The project owner's direct contacts, authors of threads and issues from the corpus (`corpus.jsonl`) **contacted through the public channels they themselves opened**, communities cited in the market track. Never compile personal contact details from multiple sources.

**Screener (5 questions, in writing).**
1. What is your role, and how many people work on web automation at your company?
2. What tools do you use today to read or act on websites (browser, HTTP, access APIs, agents)?
3. When did you last change tools, or decide to build one?
4. What is your approximate monthly budget for this (infrastructure, proxies, captchas, LLMs, services)?
5. Do you agree to a recorded 45-minute interview?

Prioritize people who have a **dated switch** to tell.

## 2. Flow (45 minutes)

**Consent (2 min).** Explain the purpose (product research, not sales), ask for agreement to record, specify that the transcript is pseudonymized (identifier `int-NN`) and that client company names are not recorded. No product demonstration during the interview.

**Context (5 min).**
- Tell me what you automate on the web, with an example from last week.
- Who depends on it, and what happens when it breaks?

**Timeline of the switch (20 min).** Trace back the last switch, step by step. Look for dates, places, people, figures.

| Stage | Questions |
| --- | --- |
| First thought | When did you first think that the previous tool was no longer enough? What had happened that day? |
| Passive looking | What did you notice next, without actively looking? |
| Active looking | What did you compare? What did you try? What eliminated each option? |
| Decision | Who decided? What tipped the balance? What did you have to justify, and to whom? |
| First use | What disappointed or surprised you during the first weeks? |
| Today | What do you miss about the previous tool? What would make you leave again? |

**The four forces (10 min).** Go over the timeline again with the person to classify what they said:
- **Push** (what made the situation untenable): blocks, bills, false successes, maintenance, lost sessions;
- **Pull** (what the new solution promised);
- **Anxiety** (what was frightening about the switch): lock-in, data, compliance, unpredictable cost;
- **Habit** (what held them back): existing code, team skills, contracts.

**Follow-ups per decision in the register (5 min, only if not raised spontaneously).**

| Decision | Factual follow-up |
| --- | --- |
| `D-feature-priority` | Tell me about the last time a collection "succeeded" with wrong or incomplete data. How did you find out? |
| `D-feature-priority`, `D-identity` | Tell me about the last logged-in session that was lost or blocked. What did you do next? |
| `D-ai-layer` | How much does an agent run cost you today, and how do you know? Have you ever frozen a flow so that you no longer call the model? |
| `D-agent-interface` | How does your agent call its tools: MCP, CLI, SDK, generated code? What did you try before? |
| `D-identity` | Has a site ever asked you to identify yourself, or blocked your declared bot? What did you do? |
| Compliance (`D-segment`) | What did your legal or security team ask for before going to production? |

**Closing (3 min).** Is there a question I should have asked? May we contact you again for a price test? If the person agrees, continue with the [pricing questionnaire](pricing-questionnaire.md).

## 3. Pitfalls to avoid

- Do not present Goodfetch before the end: the interview is about their story.
- Do not ask "would you use…" or "would you pay…": ask what happened and how much it cost.
- Follow up on every generality ("it's often slow") with a specific case ("when was the last time?").
- Record figures as stated, with their unit (per month, per page, per task).

## 4. Coding and reporting

- Pseudonymized transcript under `research/market/interviews/int-NN.md` (no names of people or client companies).
- Each passage coded with the leaf codes of the [codebook](codebook.md), plus four interview-specific codes: `force-push`, `force-pull`, `force-anxiety`, `force-habit`.
- A second independent coding on a third of the interviews. Each code is marked present or absent per passage, which allows a Cohen's kappa per code family; the target is κ ≥ 0.7, and a family below that is recoded after refining the definitions. The multi-label desk research track had only measured raw agreement ([codebook](codebook.md), §2).
- Reporting by segment: dominant forces, dated triggers, real budgets, competitor elimination criteria. Each finding cites its interviews (`int-NN`) and updates the `evidence` line of the decisions concerned.

**Decision threshold for `D-segment`.** A segment is retained if it gathers at least three interviews with a dated trigger linked to a mechanism measured in the lab (false success, session, replay cost, access policy) and a real budget already spent on this item.
