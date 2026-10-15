# A task for a developer using Goodfetch's SDK

You are a developer writing a program with Goodfetch's SDK, a library for browser automation (Collie) and crawling (Foxhound). You have not seen this SDK before: its API exists only as type stubs with doc comments, in the read-only folder `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch`. Learn the API from those files.

## Language

Python 3.11. The SDK's packages are `goodfetch_collie` (and `goodfetch_collie.testing`), `goodfetch_foxhound` (and `goodfetch_foxhound.testing`) and `goodfetch_collie_client`; their stubs are the `.pyi` files under `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch/python/`. Also installed: pydantic 2.14, fastapi 0.143, pytest 9.1 and playwright 1.64. Your program is the module `scenarios.s09_ai_in_playwright_program`.

## The task

> You have an existing Playwright program: it launches Chromium, opens https://books.toscrape.com/, clicks the "Travel" category, then clicks the first book and prints its title before closing the browser itself. Without rewriting it, add one step on the page it already drives, right after the "Travel" click: with a language model, extract the title and price of every book listed. The existing program then carries on with its page as before. The added step must not close, navigate or take over anything the program opened, and if it fails, the program must still be able to continue and close its browser.

The existing program is already in `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-py-09/scenarios/s09_ai_in_playwright_program.py`: add the step to it there.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-py-09`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-py-09/scenarios/s09_ai_in_playwright_program.py`.
- Read nothing outside your working folder and the library folder named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-py-09/check` (pyright in strict mode and mypy `--strict`). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
