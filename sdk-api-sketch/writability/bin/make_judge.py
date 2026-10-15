#!/usr/bin/env python3
"""Write one judge prompt per scenario: a fresh judge scores the behaviour of every final program of
that scenario against the scenario's intent.

Usage: make_judge.py PHASE MODELS...   (e.g. `make_judge.py a sonnet haiku`, then `make_judge.py b local`)
Phase a derives the scenario's requirements and writes them with its verdicts to WORK/judge/NN.a.json;
a later phase reuses those requirements, so every model is held to the same list. Run from WORK/bin.
"""

import json
import os
import sys
from pathlib import Path

T = Path(__file__).resolve().parent.parent
REPO = Path(os.environ.get("WRITABILITY_REPO", "/Users/jb/Dev/automan/.claude/worktrees/prototype-sdk-api-writability/sdk-api-sketch"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from make_runs import LEADERS, SLUGS, gf_program, golden_path  # noqa: E402

NOTES = {
    "03": "The task's own Success check (the address starts with https://quotes.toscrape.com/author/) is coarse: a wrong step that ends on another author's page also passes it. Do not mark a program down for using exactly that check; the golden programs do too.",
    "09": "In Rust there is no Playwright: the Rust programs were given an existing program that launches Chromium with a debugging port and were asked to write the page steps through the SDK as well as the added step.",
    "10": "The Scrapy leader program cannot use a language model; Scrapy has no AI extraction. Judge the leader programs on the task as stated: a third test the library cannot express is a missed requirement, noted as such.",
}


def leader_reference(nn: str) -> list[str]:
    out = []
    for e in LEADERS:
        if e[0] == nn:
            p = REPO / "scenarios/leaders" / ("stagehand3/" + e[3] if e[2] == "ld3" else e[3])
            out.append(str(p))
    return out


def finished(rundir: Path) -> bool:
    """A run is judged when its agent finished: a Claude run always did; a local-model run did when
    its result exists and did not end on a harness or ollama error."""
    if not rundir.name.startswith("local-"):
        return True
    result = rundir / ".agent/result.json"
    return result.exists() and not json.loads(result.read_text())["end"].startswith("error")


def programs(nn: str, models: list[str]) -> list[tuple[str, str]]:
    out = []
    for m in models:
        for lang in ("ts", "py", "rs"):
            run = f"{m}-gf-{lang}-{nn}"
            if not finished(T / "runs" / run):
                continue
            out.append((run, str(T / "runs" / run / gf_program(nn, lang))))
        for d in sorted((T / "runs").glob(f"{m}-ld-*-{nn}-*")):
            if not finished(d):
                continue
            task = json.loads((d / "task.json").read_text())
            out.append((d.name, str(d / task["program"])))
    return out


def main() -> None:
    phase, models = sys.argv[1], sys.argv[2:]
    jdir = T / "judge"
    jdir.mkdir(exist_ok=True)
    for nn, slug in SLUGS.items():
        progs = programs(nn, models)
        goldens = [str(golden_path(nn, lang)) for lang in ("ts", "py", "rs")]
        out = jdir / f"{nn}.{phase}.json"
        prior = jdir / f"{nn}.a.json"
        if phase == "a":
            req_text = (
                "First derive the scenario's requirements from its task statement (and its failure points where the "
                "task statement implies them): a short numbered list R1, R2, ... of things a correct program must do, "
                "each checkable by reading a program. Score every program against that list."
            )
        else:
            req_text = (
                f"Use exactly the requirements already derived for this scenario, in the `requirements` field of `{prior}` "
                "(do not read its verdicts), and score every program against that list."
            )
        prog_lines = "\n".join(f"- `{run}`: `{path}`" for run, path in progs)
        given = ""
        if nn in ("07", "10"):
            given = (
                "\nThe programs build on scenario 05's program (and, for 10, scenario 08's), which sat beside them in "
                "their run folder (same folder as the program); read them there if needed.\n"
            )
        prompt = f"""# Judge: golden scenario {nn} ({slug})

You are a fresh, impartial judge of programs written by coding agents. Each program attempts the same task: golden scenario {nn} of Goodfetch's SDK API sketch. Goodfetch's SDK exists only as type stubs with doc comments, so judge a Goodfetch program by what its calls would do **according to the stubs' doc comments**, and a leader program (Playwright, Stagehand, Crawlee, Scrapy, Browser Use...) by what its library really does.

## Read

- The scenario's description, with its task statement (the agents' prompt, word for word), failure points and leader: `{REPO / 'scenarios' / f'{nn}-{slug}.md'}`.
- The golden programs, written by the API's designers, as a reference for one correct answer (other correct answers exist): {', '.join(f'`{g}`' for g in goldens)}; the leader programs written for the scenario, likewise: {', '.join(f'`{p}`' for p in leader_reference(nn))}.
- The SDK's stubs, to check what a call does: `{T / 'sketch'}` (TypeScript `.d.ts`, Python `.pyi`, Rust `src/`). For a leader library, its installed package under `{T / 'ld/node_modules'}`, `{T / 'ld3/node_modules'}` or `{T / 'env/py'}/<env>/lib/python3.11/site-packages`.
- The programs to judge (each run's final program; its run folder also holds `task.json` and `prompt.md`):
{prog_lines}
{given}
## Score

{req_text}

{NOTES.get(nn, '')}

**Site facts are not scored in the verdict.** No agent could see the target pages (they had no network), so a wrong CSS selector, URL pattern, form field name or page structure of toscrape.com is a *site error*, not a behaviour failure: judge the verdict as if the program's site facts were right, and list its site errors separately. Everything else counts: the library's calls and their semantics, waits, error handling, closing, the task's logic and every requirement.

For each program give:
- `verdict`: `correct` (meets every requirement), `partial` (does the main task but misses or mishandles at least one requirement), or `wrong` (the main result would not be reached: wrong or missing calls, a step that would fail or do something else at run time, or the task largely not attempted). Type-check status is scored elsewhere; judge behaviour only.
- `missed`: the requirement IDs it misses or mishandles.
- `api_misuses`: for a Goodfetch program, each call or shape that type-checks but, by the doc comments, does not do what the program assumes (name the member, what the program assumed, what the doc comment says); also invented members that would not exist. For a leader program, the same against its library. Empty if none.
- `site_errors`: wrong site facts (selector, URL pattern, page structure), each in a few words. Empty if none.
- `notes`: one or two sentences.

Do not reward length or comments; judge what the program does. Be consistent across languages and models: the same mistake gets the same verdict.

## Output

Write a JSON file to `{out}` with this shape, and nothing else:

```json
{{"scenario": "{nn}", "requirements": [{{"id": "R1", "text": "..."}}], "programs": [{{"run": "<run name>", "verdict": "correct|partial|wrong", "missed": ["R2"], "api_misuses": [{{"member": "...", "assumed": "...", "doc_says": "..."}}], "site_errors": ["..."], "notes": "..."}}]}}
```

Every program listed above must appear once in `programs`. Read only the files named here (no web, no other runs). Reply with one line: `JUDGED {nn} <count of correct>/<count of programs>`.
"""
        (jdir / f"{nn}.{phase}.prompt.md").write_text(prompt)
    print("judge prompts written:", phase, models)


if __name__ == "__main__":
    main()
