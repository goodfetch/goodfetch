# A task for a developer using Goodfetch's SDK

You are a developer writing a program with Goodfetch's SDK, a library for browser automation (Collie) and crawling (Foxhound). You have not seen this SDK before: its API exists only as type stubs with doc comments, in the read-only folder `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch`. Learn the API from those files.

## Language

Python 3.11. The SDK's packages are `goodfetch_collie` (and `goodfetch_collie.testing`), `goodfetch_foxhound` (and `goodfetch_foxhound.testing`) and `goodfetch_collie_client`; their stubs are the `.pyi` files under `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch/python/`. Also installed: pydantic 2.14, fastapi 0.143, pytest 9.1 and playwright 1.64. Your program is the module `scenarios.s08_ai_in_crawl_handler`.

## The task

> Crawl the first three categories in the side list of https://books.toscrape.com/ and every book they list, following their "next" pages. Extract each book's title, price, number in stock and star rating with a language model instead of hand-written selectors. The model is asked once per page type: later books of the same layout are extracted from what was learned, with no model call, unless a page no longer matches what was learned. Stop all model use once the crawl has spent a set amount (for example 0.50 USD), let the crawl finish without the model, and report which books were left unextracted and why. Report the tokens and cost spent.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-gf-py-08`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-gf-py-08/scenarios/s08_ai_in_crawl_handler.py`.
- Read nothing outside your working folder and the library folder named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-gf-py-08/check` (pyright in strict mode and mypy `--strict`). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
