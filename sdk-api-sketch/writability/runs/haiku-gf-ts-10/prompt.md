# A task for a developer using Goodfetch's SDK

You are a developer writing a program with Goodfetch's SDK, a library for browser automation (Collie) and crawling (Foxhound). You have not seen this SDK before: its API exists only as type stubs with doc comments, in the read-only folder `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch`. Learn the API from those files.

## Language

TypeScript, as an ES module run by Node 22 or later (top-level `await` and `await using` work). The SDK's packages are `@goodfetch/collie` (and `@goodfetch/collie/testing`), `@goodfetch/foxhound` (and `@goodfetch/foxhound/testing`) and `@goodfetch/collie-client` (and its `/testing`); their stubs are the `.d.ts` files under `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch/typescript/`. Also installed: zod 4.6.7, playwright 1.64.1 and @types/node. Import another local module with a `.js` suffix.

## The task

> Write tests for the crawl of scenario 05 that run with no network and no model, with the language's usual test runner. First, a unit test of the book-page handler fed a saved copy of one book page, checking the item it produces, including the category passed to it. Second, a test that runs the whole crawl from the responses recorded during one earlier live run, and checks that the dataset holds the expected number of books. Third, for the AI extraction of scenario 08, a test that runs from what was learned on the earlier run, or from a scripted model reply, and fails if a real model is called. Any test that would reach the network must fail, saying which address it tried.

The programs of the scenarios this task builds on are already in your working folder, written against the same SDK: `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-10/05-two-page-crawl.ts`, `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-10/08-ai-in-crawl-handler.ts`. Import from them; do not change them.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-10`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-10/10-offline-crawl-tests.ts`.
- Read nothing outside your working folder and the library folder named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-10/check` (tsc 7 with `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax` and `erasableSyntaxOnly`). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
