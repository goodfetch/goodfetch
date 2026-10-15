#!/usr/bin/env sh
# Type-checks the SDK API sketch in all three languages, then the leaders' programs of the golden
# scenarios against their real packages. Needs Node 22+, uv and a Rust toolchain.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
leaders="$here/scenarios/leaders"

# Scenario 09 adds a step to a program of the user's own on Playwright, so Playwright is a dev
# dependency of the scenarios, never of the stubs (G7). Its browsers are not downloaded.
echo "TypeScript (tsc 7.0.2, zod 4.6.7, @types/node 24.19.2, playwright 1.64.1)"
(cd "$here/typescript" && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install --no-audit --no-fund --silent \
  && npx --yes -p typescript@7.0.2 tsc -p .)

# The scenarios use FastAPI (12), pytest (10) and Playwright (09, the user's own program) beside the stubs.
echo "Python (pyright 1.1.414, mypy 2.4.0, pydantic 2.14.1, fastapi 0.143.1, pytest 9.1.1, playwright 1.64.0)"
py_deps="--with pydantic==2.14.1 --with fastapi==0.143.1 --with pytest==9.1.1 --with playwright==1.64.0"
# shellcheck disable=SC2086 # the options are split on purpose
(cd "$here/python" && uv run --no-project $py_deps --with pyright==1.1.414 pyright \
  && uv run --no-project $py_deps --with mypy==2.4.0 mypy)

echo "Rust (cargo check)"
(cd "$here/rust" && RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --all-features --quiet)

# The leaders' programs are type-checked, never run (scenarios/README.md). Browsers are not downloaded.
# Zod is pinned at 4.4.3 there, the exact version Stagehand 4.2.1 depends on: a newer copy's schemas
# fail `extract`'s types (`_zod.version.minor`), so the leader is checked as its users install it.
echo "Leaders, TypeScript (tsc 7.0.2: Playwright 1.64.1, Stagehand 4.2.1, Crawlee 3.18.2, zod 4.4.3)"
(cd "$leaders" && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install --no-audit --no-fund --silent \
  && npx --yes -p typescript@7.0.2 tsc -p .)
echo "Leaders, TypeScript (tsc 7.0.2: Stagehand 3.7.3, @crawlee/stagehand 3.18.2)"
(cd "$leaders/stagehand3" && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install --no-audit --no-fund --silent \
  && npx --yes -p typescript@7.0.2 tsc -p .)

# One environment per leader, since their pins disagree; pyright runs from the leaders' folder,
# so a test imports the spider it tests by module name.
leader_py() {
  files=$1
  shift
  # shellcheck disable=SC2086 # the file list is split on purpose
  (cd "$leaders" && uv run --no-project --python 3.11 "$@" --with pyright==1.1.414 pyright $files)
}
echo "Leaders, Python (pyright 1.1.414)"
leader_py "s01_dependent_form_playwright.py s04_login_human_pause_playwright.py" --with playwright==1.64.0
leader_py "s05_two_page_crawl_scrapy.py s07_resume_and_retry_scrapy.py s10_offline_crawl_tests_scrapy.py s12_crawl_in_a_service_scrapy.py" \
  --with scrapy==2.19.0 --with pytest==9.1.1 --with fastapi==0.143.1 --with uvicorn==0.54.0
leader_py "s06_mixed_fetch_crawl_scrapy_playwright.py" --with scrapy==2.19.0 --with scrapy-playwright==0.0.48
leader_py "s08_ai_in_crawl_handler_crawlee.py" --with "crawlee[pydantic-ai,playwright]==1.11.0"
leader_py "s11_agent_goal_browser_use.py" --with browser-use==0.13.11

echo "All three type-check, and so do the leaders' programs."
