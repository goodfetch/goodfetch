# A task for a developer using Stagehand 4.2.1

You are a developer writing a program with Stagehand 4.2.1 (`@browserbasehq/stagehand`), with zod 4.4.3.

## Language

TypeScript, as an ES module run by Node 22 or later (top-level `await` works), with Stagehand 4.2.1 (`@browserbasehq/stagehand`), with zod 4.4.3. The packages are installed in `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-ld-ts-02-stagehand/node_modules`, with their type definitions, and @types/node. Import another local module with a `.js` suffix.

## The task

> From https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html, extract the book as a typed object: its title (text), its price (a decimal number of pounds), the number of copies in stock (an integer), its star rating (one of one to five) and its description (text). Use a language model to read the page instead of hand-written selectors. The program chooses the model by name, and a model served locally must work as well as a hosted one. Print the tokens used and their cost next to the result. If a field cannot be found or does not have the expected type, say which field, instead of returning an empty or wrong value.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-ld-ts-02-stagehand`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-ld-ts-02-stagehand/02-typed-extract.stagehand.ts`.
- Read nothing outside your working folder and the installed packages named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-ld-ts-02-stagehand/check` (tsc 7 with `strict` (and `skipLibCheck`)). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
