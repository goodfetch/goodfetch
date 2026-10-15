#!/usr/bin/env python3
"""Score the writability runs from what is committed in writability/runs and writability/judgments.

Usage: score.py   (writes writability/results.json and writability/results-table.md)

Per run:
- type-check at first attempt: the first check run (the program as it was when the agent first
  checked it, or, for an agent that never checked, its final program checked by the harness);
- type-check at the end: the last check run;
- behaviour: the judge's verdict (correct, partial, wrong) on the final program, site facts excluded;
- first-attempt correct: type-checks at first attempt, the final program is that first attempt
  unchanged, and the judge found it correct;
- correct in the end: type-checks at the end and the judge found it correct.
A local-model run that never started, was stopped or ended on an ollama error is "not run" and
counts nowhere.
"""

import json
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
RUNS = HERE / "runs"
JUDGE = HERE / "judgments"
MODELS = {"sonnet": "Claude Sonnet", "haiku": "Claude Haiku", "local": "qwen2.5-coder:14b (local)"}
SCENARIOS = [f"{n:02d}" for n in range(1, 13)]


def verdicts() -> dict[str, dict]:
    out = {}
    for f in sorted(JUDGE.glob("*.json")):
        if f.name.count(".") != 2:  # NN.a.json, NN.b.json
            continue
        for p in json.loads(f.read_text())["programs"]:
            out[p["run"]] = p
    return out


def status(d: Path) -> str:
    if not d.name.startswith("local-"):
        return "run"
    if (d / ".agent/stopped.json").exists():
        return "stopped"
    r = d / ".agent/result.json"
    if not r.exists():
        return "not run"
    if json.loads(r.read_text())["end"].startswith("error"):
        return "ollama error"
    return "run"


def score() -> list[dict]:
    judged = verdicts()
    rows = []
    for d in sorted(RUNS.iterdir()):
        if not (d / "task.json").exists():
            continue
        task = json.loads((d / "task.json").read_text())
        row = {
            "run": d.name,
            "model": task["model"],
            "kind": task["kind"],
            "lang": task["lang"],
            "scenario": task["scenario"],
            "library": task.get("lib", "Goodfetch"),
            "status": status(d),
        }
        if row["status"] == "run":
            checks = sorted((d / ".checks").glob("run-*.json"), key=lambda p: int(p.stem.split("-")[1]))
            first, last = json.loads(checks[0].read_text()), json.loads(checks[-1].read_text())
            flat = task["program"].replace("/", "_")
            first_prog = (d / ".checks" / f"first-{flat}").read_text() if (d / ".checks" / f"first-{flat}").exists() else None
            final_prog = (d / task["program"]).read_text() if (d / task["program"]).exists() else None  # never written
            j = judged.get(d.name)
            row.update(
                checks=len(checks),
                checked_by=first.get("by", "agent"),
                tc_first=first["pass"],
                tc_final=last["pass"],
                lenient_first=first.get("lenient_pass", first["pass"]),
                unchanged=first_prog == final_prog,
                verdict=j["verdict"] if j else None,
                missed=j["missed"] if j else None,
                api_misuses=len(j["api_misuses"]) if j else None,
                site_errors=len(j.get("site_errors", [])) if j else None,
            )
            row["first_correct"] = bool(row["tc_first"] and row["unchanged"] and row["verdict"] == "correct")
            row["final_correct"] = bool(row["tc_final"] and row["verdict"] == "correct")
        rows.append(row)
    return rows


def pct(a: int, b: int) -> str:
    return f"{a}/{b}" + (f" ({round(100 * a / b)}%)" if b else "")


def summary(rows: list[dict], pred) -> dict[str, int]:
    sel = [r for r in rows if pred(r) and r["status"] == "run"]
    return {
        "runs": len(sel),
        "tc_first": sum(r["tc_first"] for r in sel),
        "tc_final": sum(r["tc_final"] for r in sel),
        "correct": sum(r["verdict"] == "correct" for r in sel),
        "partial": sum(r["verdict"] == "partial" for r in sel),
        "wrong": sum(r["verdict"] == "wrong" for r in sel),
        "first_correct": sum(r["first_correct"] for r in sel),
        "final_correct": sum(r["final_correct"] for r in sel),
    }


def cell(r: dict | None) -> str:
    if r is None:
        return ""
    if r["status"] != "run":
        return {"not run": "not run", "stopped": "stopped", "ollama error": "ollama error"}[r["status"]]
    tc = ("✓" if r["tc_first"] else "✗") + ("" if r["tc_first"] == r["tc_final"] else ("→✓" if r["tc_final"] else "→✗"))
    v = {"correct": "C", "partial": "P", "wrong": "W", None: "?"}[r["verdict"]]
    return f"{tc} {v}" + (" ★" if r["first_correct"] else "")


def table(rows: list[dict]) -> str:
    by = {r["run"]: r for r in rows}
    out = []
    out.append("## Goodfetch programs, per scenario and language\n")
    out.append("| # | Language | Sonnet | Haiku | qwen2.5-coder:14b |")
    out.append("|---|---|---|---|---|")
    for nn in SCENARIOS:
        for lang in ("ts", "py", "rs"):
            out.append(
                f"| {nn} | {lang} | "
                + " | ".join(cell(by.get(f"{m}-gf-{lang}-{nn}")) for m in MODELS)
                + " |"
            )
    out.append("\n## Leader programs (the same task on the leader's API)\n")
    out.append("| # | Leader | Language | Sonnet | Haiku | qwen2.5-coder:14b |")
    out.append("|---|---|---|---|---|---|")
    leaders = sorted({(r["scenario"], r["lang"], r["run"].split("-", 2)[2]) for r in rows if r["kind"] == "leader"})
    for nn, lang, rest in leaders:
        lib = next(r["library"] for r in rows if r["run"].endswith(rest) and r["kind"] == "leader")
        out.append(
            f"| {nn} | {lib.split(' (')[0]} | {lang} | "
            + " | ".join(cell(by.get(f"{m}-ld-{rest}")) for m in MODELS)
            + " |"
        )
    return "\n".join(out)


def main() -> None:
    rows = score()
    leader_pairs = {(r["scenario"], r["lang"]) for r in rows if r["kind"] == "leader"}
    sums = {}
    for m in MODELS:
        sums[m] = {
            "goodfetch_all": summary(rows, lambda r, m=m: r["model"] == m and r["kind"] == "gf"),
            "goodfetch_by_lang": {
                lang: summary(rows, lambda r, m=m, lang=lang: r["model"] == m and r["kind"] == "gf" and r["lang"] == lang)
                for lang in ("ts", "py", "rs")
            },
            "goodfetch_on_leader_tasks": summary(
                rows, lambda r, m=m: r["model"] == m and r["kind"] == "gf" and (r["scenario"], r["lang"]) in leader_pairs
            ),
            "leaders": summary(rows, lambda r, m=m: r["model"] == m and r["kind"] == "leader"),
        }
    (HERE / "results.json").write_text(json.dumps({"summary": sums, "runs": rows}, indent=1, ensure_ascii=False))
    lines = ["# Results table", "", "Generated by `bin/score.py` from `runs/` and `judgments/`; see README.md for the method.", ""]
    lines.append("## Summary\n")
    lines.append(
        "| Model | Programs | Runs | Type-checks, first attempt | Type-checks, end | Behaviour correct / partial / wrong | First-attempt correct | Correct in the end |"
    )
    lines.append("|---|---|---|---|---|---|---|---|")
    labels = {
        "goodfetch_all": "Goodfetch, all 36",
        "goodfetch_on_leader_tasks": "Goodfetch, the 18 leader tasks",
        "leaders": "Leaders, the 18 tasks",
    }
    for m, name in MODELS.items():
        for key, label in labels.items():
            s = sums[m][key]
            if not s["runs"]:
                lines.append(f"| {name} | {label} | 0 | | | | | |")
                continue
            lines.append(
                f"| {name} | {label} | {s['runs']} | {pct(s['tc_first'], s['runs'])} | {pct(s['tc_final'], s['runs'])} | "
                f"{s['correct']} / {s['partial']} / {s['wrong']} | {pct(s['first_correct'], s['runs'])} | {pct(s['final_correct'], s['runs'])} |"
            )
    lines.append("\n### Goodfetch by language\n")
    lines.append("| Model | Language | Runs | Type-checks, first attempt | Behaviour correct / partial / wrong | First-attempt correct |")
    lines.append("|---|---|---|---|---|---|")
    for m, name in MODELS.items():
        for lang, s in sums[m]["goodfetch_by_lang"].items():
            if s["runs"]:
                lines.append(
                    f"| {name} | {lang} | {s['runs']} | {pct(s['tc_first'], s['runs'])} | {s['correct']} / {s['partial']} / {s['wrong']} | {pct(s['first_correct'], s['runs'])} |"
                )
    lines.append(
        "\nCell key: type-check at first attempt (✓/✗, then →✓/→✗ where the end differs), then the judge's behaviour verdict "
        "(C correct, P partial, W wrong); ★ first-attempt correct. \"not run\", \"stopped\": local-model runs stopped at the "
        "owner's request; \"ollama error\": the local model was removed mid-run. None of those counts.\n"
    )
    lines.append(table(rows))
    (HERE / "results-table.md").write_text("\n".join(lines) + "\n")
    for m in MODELS:
        print(m, json.dumps(sums[m]["goodfetch_all"]), json.dumps(sums[m]["goodfetch_on_leader_tasks"]), json.dumps(sums[m]["leaders"]))


if __name__ == "__main__":
    main()
