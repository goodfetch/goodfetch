# A task for a developer using Crawlee 3.18.2 with Stagehand 3.7.3

You are a developer writing a program with Crawlee 3.18.2 with Stagehand 3.7.3 (`crawlee`, `@crawlee/stagehand` 3.18.2, `@browserbasehq/stagehand` 3.7.3), with zod 4.

## Language

TypeScript, as an ES module run by Node 22 or later (top-level `await` works), with Crawlee 3.18.2 with Stagehand 3.7.3 (`crawlee`, `@crawlee/stagehand` 3.18.2, `@browserbasehq/stagehand` 3.7.3), with zod 4. The packages are installed in `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-ld-ts-08-crawlee-stagehand/node_modules`, with their type definitions, and @types/node. Import another local module with a `.js` suffix.

## The task

> Crawl the first three categories in the side list of https://books.toscrape.com/ and every book they list, following their "next" pages. Extract each book's title, price, number in stock and star rating with a language model instead of hand-written selectors. The model is asked once per page type: later books of the same layout are extracted from what was learned, with no model call, unless a page no longer matches what was learned. Stop all model use once the crawl has spent a set amount (for example 0.50 USD), let the crawl finish without the model, and report which books were left unextracted and why. Report the tokens and cost spent.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-ld-ts-08-crawlee-stagehand`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-ld-ts-08-crawlee-stagehand/08-ai-in-crawl-handler.crawlee-stagehand.ts`.
- Read nothing outside your working folder and the installed packages named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-ld-ts-08-crawlee-stagehand/check` (tsc 7 with `strict` (and `skipLibCheck`)). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
