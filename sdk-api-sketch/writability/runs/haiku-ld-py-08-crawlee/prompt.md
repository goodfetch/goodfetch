# A task for a developer using Crawlee for Python 1.11.0 with its PydanticAI and Playwright extras

You are a developer writing a program with Crawlee for Python 1.11.0 with its PydanticAI and Playwright extras (`crawlee[pydantic-ai,playwright]`).

## Language

Python 3.11, with Crawlee for Python 1.11.0 with its PydanticAI and Playwright extras (`crawlee[pydantic-ai,playwright]`). The packages are installed in `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/env/py/crawlee/lib/python3.11/site-packages`, if you need to read their sources.

## The task

> Crawl the first three categories in the side list of https://books.toscrape.com/ and every book they list, following their "next" pages. Extract each book's title, price, number in stock and star rating with a language model instead of hand-written selectors. The model is asked once per page type: later books of the same layout are extracted from what was learned, with no model call, unless a page no longer matches what was learned. Stop all model use once the crawl has spent a set amount (for example 0.50 USD), let the crawl finish without the model, and report which books were left unextracted and why. Report the tokens and cost spent.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-py-08-crawlee`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-py-08-crawlee/s08_ai_in_crawl_handler_crawlee.py`.
- Read nothing outside your working folder and the installed packages named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-py-08-crawlee/check` (pyright in standard mode). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
