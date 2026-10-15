#!/usr/bin/env python3
"""Create the writability runs: one folder per (model, scenario, language), and one per leader program.

Usage: make_runs.py MODEL [--golden]
  MODEL is sonnet, haiku or qwen; --golden copies the golden and leader programs in instead, to
  validate the checker. Run from the work folder's copy (WORK/bin); runs go to WORK/runs. The
  sketch's checkout is $WRITABILITY_REPO (default: this worktree's sdk-api-sketch).
"""

import json
import os
import re
import shutil
import sys
from pathlib import Path

T = Path(__file__).resolve().parent.parent
SKETCH_REPO = Path(
    os.environ.get(
        "WRITABILITY_REPO", "/Users/jb/Dev/automan/.claude/worktrees/prototype-sdk-api-writability/sdk-api-sketch"
    )
)
RUNS = T / "runs"
SKETCH = T / "sketch"

SLUGS = {
    "01": "dependent-form",
    "02": "typed-extract",
    "03": "replay-ai-steps",
    "04": "login-human-pause",
    "05": "two-page-crawl",
    "06": "mixed-fetch-crawl",
    "07": "resume-and-retry",
    "08": "ai-in-crawl-handler",
    "09": "ai-in-playwright-program",
    "10": "offline-crawl-tests",
    "11": "agent-goal",
    "12": "crawl-in-a-service",
}

# (scenario, language, environment, file name, library) for the 18 leader programs of scenarios/README.md.
PLAYWRIGHT_TS = "Playwright 1.64.1 (the `playwright` package)"
PLAYWRIGHT_PY = "Playwright for Python 1.64.0 (`playwright`)"
STAGEHAND4 = "Stagehand 4.2.1 (`@browserbasehq/stagehand`), with zod 4.4.3"
CRAWLEE = "Crawlee 3.18.2 (`crawlee`)"
SCRAPY = "Scrapy 2.19.0 (`scrapy`)"
LEADERS = [
    ("01", "ts", "ld", "01-dependent-form.playwright.ts", PLAYWRIGHT_TS),
    ("01", "py", "pw", "s01_dependent_form_playwright.py", PLAYWRIGHT_PY),
    ("02", "ts", "ld", "02-typed-extract.stagehand.ts", STAGEHAND4),
    ("03", "ts", "ld", "03-replay-ai-steps.stagehand.ts", STAGEHAND4),
    ("04", "ts", "ld", "04-login-human-pause.playwright.ts", PLAYWRIGHT_TS),
    ("04", "py", "pw", "s04_login_human_pause_playwright.py", PLAYWRIGHT_PY),
    ("05", "ts", "ld", "05-two-page-crawl.crawlee.ts", CRAWLEE),
    ("05", "py", "scrapy", "s05_two_page_crawl_scrapy.py", SCRAPY),
    ("06", "ts", "ld", "06-mixed-fetch-crawl.crawlee.ts", "Crawlee 3.18.2 (`crawlee`, with `playwright` 1.64.1)"),
    ("06", "py", "scrapypw", "s06_mixed_fetch_crawl_scrapy_playwright.py", "Scrapy 2.19.0 with scrapy-playwright 0.0.48"),
    ("07", "ts", "ld", "07-resume-and-retry.crawlee.ts", CRAWLEE),
    ("07", "py", "scrapy", "s07_resume_and_retry_scrapy.py", SCRAPY),
    (
        "08",
        "ts",
        "ld3",
        "08-ai-in-crawl-handler.crawlee-stagehand.ts",
        "Crawlee 3.18.2 with Stagehand 3.7.3 (`crawlee`, `@crawlee/stagehand` 3.18.2, `@browserbasehq/stagehand` 3.7.3), with zod 4",
    ),
    (
        "08",
        "py",
        "crawlee",
        "s08_ai_in_crawl_handler_crawlee.py",
        "Crawlee for Python 1.11.0 with its PydanticAI and Playwright extras (`crawlee[pydantic-ai,playwright]`)",
    ),
    (
        "09",
        "ts",
        "ld3",
        "09-ai-in-playwright-program.stagehand.ts",
        "Stagehand 3.7.3 (`@browserbasehq/stagehand`) on the program's Playwright 1.64.1, with zod 4",
    ),
    ("10", "py", "scrapy", "s10_offline_crawl_tests_scrapy.py", "Scrapy 2.19.0, tested with pytest 9.1.1"),
    ("11", "py", "bu", "s11_agent_goal_browser_use.py", "Browser Use 0.13.11 (`browser_use`)"),
    ("12", "py", "scrapy", "s12_crawl_in_a_service_scrapy.py", "Scrapy 2.19.0 with FastAPI 0.143.1 (and uvicorn 0.54.0)"),
]
LEADER_GIVEN = {
    ("07", "ts"): ["05-two-page-crawl.crawlee.ts"],
    ("07", "py"): ["s05_two_page_crawl_scrapy.py"],
    ("10", "py"): ["s05_two_page_crawl_scrapy.py"],
}
PY_ENV_SLUG = {"pw": "playwright", "scrapy": "scrapy", "scrapypw": "scrapy_playwright", "crawlee": "crawlee", "bu": "browser_use"}

# Scenario 09's existing program: the golden program without its added step.
EXISTING_09_TS = """import { chromium } from "playwright";

const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto("https://books.toscrape.com/");
await page.getByRole("link", { name: "Travel" }).click();
await page.locator("article.product_pod h3 a").first().click();
console.log(await page.locator(".product_main h1").textContent());
await browser.close();
"""

EXISTING_09_PY = """import asyncio

from playwright.async_api import async_playwright


async def main() -> None:
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch()
        page = await browser.new_page()
        await page.goto("https://books.toscrape.com/")
        await page.get_by_role("link", name="Travel").click()
        await page.locator("article.product_pod h3 a").first.click()
        print(await page.locator(".product_main h1").text_content())
        await browser.close()


asyncio.run(main())
"""

# Rust has no Playwright (scenario 09's Setup): the existing program launches Chromium with a debugging port.
EXISTING_09_RS = """//! The existing program. Rust has no Playwright: this program launches Chromium itself (the
//! executable is its first argument) with a remote debugging port, 127.0.0.1:9222, and drives
//! that browser's page through the SDK: it opens https://books.toscrape.com/, clicks the "Travel"
//! category, clicks the first book and prints its title, then closes the browser itself.

use std::process::Command;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let chromium = std::env::args().nth(1).unwrap_or_else(|| "chromium".to_owned());
    let mut browser = Command::new(chromium)
        .args(["--headless=new", "--remote-debugging-port=9222"])
        .spawn()?;

    // ... the page steps described above, driven through the SDK on this browser ...

    browser.kill()?;
    browser.wait()?;
    Ok(())
}
"""

LANG_TEXT = {
    "ts": (
        "TypeScript, as an ES module run by Node 22 or later (top-level `await` and `await using` work). "
        "The SDK's packages are `@goodfetch/collie` (and `@goodfetch/collie/testing`), `@goodfetch/foxhound` "
        "(and `@goodfetch/foxhound/testing`) and `@goodfetch/collie-client` (and its `/testing`); their stubs are "
        "the `.d.ts` files under `{sketch}/typescript/`. Also installed: zod 4.6.7, playwright 1.64.1 and "
        "@types/node. Import another local module with a `.js` suffix."
    ),
    "py": (
        "Python 3.11. The SDK's packages are `goodfetch_collie` (and `goodfetch_collie.testing`), "
        "`goodfetch_foxhound` (and `goodfetch_foxhound.testing`) and `goodfetch_collie_client`; their stubs are the "
        "`.pyi` files under `{sketch}/python/`. Also installed: pydantic 2.14, fastapi 0.143, pytest 9.1 and "
        "playwright 1.64. Your program is the module `scenarios.{module}`."
    ),
    "rs": (
        "Rust, edition 2024, as a cargo example of a package that depends on `goodfetch-collie`, "
        "`goodfetch-foxhound` (both with their `test-util` feature on) and `goodfetch-collie-client`, "
        "plus futures-core 0.3, schemars 1.2, serde 1 (derive) and serde_json 1, and as dev-dependencies tokio 1 "
        "(macros, rt-multi-thread, signal), axum 0.8 and futures-util 0.3. The crates' stubs (signatures with "
        "`unimplemented!()` bodies and doc comments) are under `{sketch}/rust/` (`collie-client/src`, "
        "`collie/src`, `foxhound/src`). The example is checked with `cargo check` with warnings denied, and "
        "with the test profile too if it holds tests; the workspace sets `missing_docs = \"warn\"`."
    ),
}
CHECKS_TEXT = {
    "ts": "tsc 7 with `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax` and `erasableSyntaxOnly`",
    "py": "pyright in strict mode and mypy `--strict`",
    "rs": "`cargo check` with warnings denied",
}
LEADER_CHECKS_TEXT = {"ts": "tsc 7 with `strict` (and `skipLibCheck`)", "py": "pyright in standard mode"}


def task_statement(nn: str) -> str:
    text = (SKETCH_REPO / "scenarios" / f"{nn}-{SLUGS[nn]}.md").read_text()
    m = re.search(r"## Task statement\n\n(.*?)\n\n## ", text, re.S)
    assert m, nn
    return m.group(1).strip()


def gf_program(nn: str, lang: str) -> str:
    snake = SLUGS[nn].replace("-", "_")
    return {"ts": f"{nn}-{SLUGS[nn]}.ts", "py": f"scenarios/s{nn}_{snake}.py", "rs": f"s{nn}_{snake}.rs"}[lang]


def golden_path(nn: str, lang: str) -> Path:
    snake = SLUGS[nn].replace("-", "_")
    rel = {
        "ts": f"typescript/scenarios/{nn}-{SLUGS[nn]}.ts",
        "py": f"python/scenarios/s{nn}_{snake}.py",
        "rs": f"rust/examples/s{nn}_{snake}.rs",
    }[lang]
    return SKETCH_REPO / rel


def how_to_work(rundir: Path, program: str, checks: str, library: str) -> str:
    return f"""## How to work

- Your working folder is `{rundir}`. Write the complete program to `{rundir / program}`.
- Read nothing outside your working folder and the {library} named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `{rundir / 'check'}` ({checks}). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
"""


def given_text(rundir: Path, given: list[str], library: str) -> str:
    if not given:
        return ""
    many = len(given) > 1
    names = ", ".join(f"`{rundir / g}`" for g in given)
    return (
        f"\nThe program{'s' if many else ''} of the scenario{'s' if many else ''} this task builds on "
        f"{'are' if many else 'is'} already in your working folder, written {library}: {names}. "
        f"Import from {'them' if many else 'it'}; do not change {'them' if many else 'it'}.\n"
    )


def fresh(rundir: Path) -> None:
    if rundir.exists():
        shutil.rmtree(rundir)
    rundir.mkdir(parents=True)


def write_check(rundir: Path, lang: str) -> None:
    if lang == "ts":
        (rundir / "package.json").write_text('{ "private": true, "type": "module" }\n')
    c = rundir / "check"
    c.write_text(f'#!/bin/sh\nexec python3 {T / "bin/check.py"} {rundir} "$@"\n')
    c.chmod(0o755)


def make_gf(model: str, nn: str, lang: str, golden: bool) -> None:
    rundir = RUNS / f"{model}-gf-{lang}-{nn}"
    fresh(rundir)
    program = gf_program(nn, lang)
    (rundir / program).parent.mkdir(parents=True, exist_ok=True)
    if lang == "py":
        (rundir / "scenarios/__init__.py").write_text("")
    given = []
    for d in {"07": ["05"], "10": ["05", "08"]}.get(nn, []):
        g = gf_program(d, lang)
        shutil.copy(golden_path(d, lang), rundir / g)
        given.append(g)
    if golden:
        shutil.copy(golden_path(nn, lang), rundir / program)
    elif nn == "09":
        (rundir / program).write_text({"ts": EXISTING_09_TS, "py": EXISTING_09_PY, "rs": EXISTING_09_RS}[lang])
    task = {"kind": "gf", "lang": lang, "program": program, "given": given, "scenario": nn, "model": model}
    (rundir / "task.json").write_text(json.dumps(task, indent=2))
    write_check(rundir, lang)
    extra = given_text(rundir, given, "against the same SDK")
    if nn == "09":
        extra = f"\nThe existing program is already in `{rundir / program}`: add the step to it there.\n"
        if lang == "rs":
            extra += (
                "Rust has no Playwright, so the existing program launches Chromium itself with a remote debugging "
                "port and drives that browser's page through the SDK; the page steps it performs are described in "
                "its comments. Write those steps and the added step.\n"
            )
    prompt = f"""# A task for a developer using Goodfetch's SDK

You are a developer writing a program with Goodfetch's SDK, a library for browser automation (Collie) and crawling (Foxhound). You have not seen this SDK before: its API exists only as type stubs with doc comments, in the read-only folder `{SKETCH}`. Learn the API from those files.

## Language

{LANG_TEXT[lang].format(sketch=SKETCH, module=Path(program).stem)}

## The task

{task_statement(nn)}
{extra}
{how_to_work(rundir, program, CHECKS_TEXT[lang], "library folder")}"""
    (rundir / "prompt.md").write_text(prompt)


def make_leader(model: str, entry: tuple[str, str, str, str, str], golden: bool) -> None:
    nn, lang, env, fname, lib = entry
    libslug = fname.split(".")[1] if lang == "ts" else PY_ENV_SLUG[env]
    rundir = RUNS / f"{model}-ld-{lang}-{nn}-{libslug}"
    fresh(rundir)
    leaders = SKETCH_REPO / "scenarios/leaders"
    given = LEADER_GIVEN.get((nn, lang), [])
    for g in given:
        shutil.copy(leaders / g, rundir / g)
    if golden:
        shutil.copy(leaders / ("stagehand3/" + fname if env == "ld3" else fname), rundir / fname)
    elif nn == "09":
        (rundir / fname).write_text(EXISTING_09_TS)
    task = {"kind": "leader", "lang": lang, "program": fname, "given": given, "env": env, "scenario": nn, "model": model, "lib": lib}
    (rundir / "task.json").write_text(json.dumps(task, indent=2))
    write_check(rundir, lang)
    if lang == "ts":
        (rundir / "node_modules").symlink_to(T / env / "node_modules")
        lang_text = (
            f"TypeScript, as an ES module run by Node 22 or later (top-level `await` works), with {lib}. "
            f"The packages are installed in `{rundir / 'node_modules'}`, with their type definitions, and "
            "@types/node. Import another local module with a `.js` suffix."
        )
    else:
        site = T / "env/py" / env / "lib/python3.11/site-packages"
        lang_text = f"Python 3.11, with {lib}. The packages are installed in `{site}`, if you need to read their sources."
    extra = given_text(rundir, given, "with the same library")
    if nn == "09":
        extra = f"\nThe existing program is already in `{rundir / fname}`: add the step to it there.\n"
    prompt = f"""# A task for a developer using {lib.split(' (')[0]}

You are a developer writing a program with {lib}.

## Language

{lang_text}

## The task

{task_statement(nn)}
{extra}
{how_to_work(rundir, fname, LEADER_CHECKS_TEXT[lang], "installed packages")}"""
    (rundir / "prompt.md").write_text(prompt)


def main() -> None:
    model = sys.argv[1]
    golden = "--golden" in sys.argv
    for nn in SLUGS:
        for lang in ("ts", "py", "rs"):
            make_gf(model, nn, lang, golden)
    for entry in LEADERS:
        make_leader(model, entry, golden)
    print(36 + len(LEADERS), "runs for", model)


if __name__ == "__main__":
    main()
