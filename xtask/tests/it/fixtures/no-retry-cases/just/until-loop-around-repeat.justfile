set shell := ["bash", "-euo", "pipefail", "-c"]

test suite *packages:
    #!/usr/bin/env bash
    set -euo pipefail
    packages=()
    for package in {{ packages }}; do packages+=(-p "$package"); done
    [[ ${#packages[@]} -eq 0 ]] && packages=(--workspace)
    NEXTEST_NO_TESTS=pass cargo insta test "${packages[@]}" --check \
      --test-runner nextest --nextest-profile ci --retries 0 --flaky-result fail
    pnpm exec vitest run --retry=0
    pnpm exec playwright test --retries=0 --pass-with-no-tests
    uv run pytest || [[ $? -eq 5 ]]
    until just _repeat 1; do sleep 1; done
