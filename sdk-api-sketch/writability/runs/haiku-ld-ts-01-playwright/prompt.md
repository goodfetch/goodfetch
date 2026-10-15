# A task for a developer using Playwright 1.64.1

You are a developer writing a program with Playwright 1.64.1 (the `playwright` package).

## Language

TypeScript, as an ES module run by Node 22 or later (top-level `await` works), with Playwright 1.64.1 (the `playwright` package). The packages are installed in `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-ts-01-playwright/node_modules`, with their type definitions, and @types/node. Import another local module with a `.js` suffix.

## The task

> Open https://quotes.toscrape.com/search.aspx. Choose the author "Albert Einstein": the page reloads and the tag list then offers that author's tags. Choose the tag "inspirational", press Search, and return every quote shown as a list of typed objects with the quote's text, its author and the tag. If no quote matches, return an empty list and say so; if the page does not behave as described, fail with an error that says which step and what was seen. Close everything the program opened before it ends.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-ts-01-playwright`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-ts-01-playwright/01-dependent-form.playwright.ts`.
- Read nothing outside your working folder and the installed packages named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-ts-01-playwright/check` (tsc 7 with `strict` (and `skipLibCheck`)). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
