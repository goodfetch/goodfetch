# A task for a developer using Goodfetch's SDK

You are a developer writing a program with Goodfetch's SDK, a library for browser automation (Collie) and crawling (Foxhound). You have not seen this SDK before: its API exists only as type stubs with doc comments, in the read-only folder `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch`. Learn the API from those files.

## Language

Python 3.11. The SDK's packages are `goodfetch_collie` (and `goodfetch_collie.testing`), `goodfetch_foxhound` (and `goodfetch_foxhound.testing`) and `goodfetch_collie_client`; their stubs are the `.pyi` files under `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch/python/`. Also installed: pydantic 2.14, fastapi 0.143, pytest 9.1 and playwright 1.64. Your program is the module `scenarios.s10_offline_crawl_tests`.

## The task

> Write tests for the crawl of scenario 05 that run with no network and no model, with the language's usual test runner. First, a unit test of the book-page handler fed a saved copy of one book page, checking the item it produces, including the category passed to it. Second, a test that runs the whole crawl from the responses recorded during one earlier live run, and checks that the dataset holds the expected number of books. Third, for the AI extraction of scenario 08, a test that runs from what was learned on the earlier run, or from a scripted model reply, and fails if a real model is called. Any test that would reach the network must fail, saying which address it tried.

The programs of the scenarios this task builds on are already in your working folder, written against the same SDK: `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-py-10/scenarios/s05_two_page_crawl.py`, `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-py-10/scenarios/s08_ai_in_crawl_handler.py`. Import from them; do not change them.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-py-10`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-py-10/scenarios/s10_offline_crawl_tests.py`.
- Read nothing outside your working folder and the library folder named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-py-10/check` (pyright in strict mode and mypy `--strict`). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
