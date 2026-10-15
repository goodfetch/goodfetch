# A task for a developer using Goodfetch's SDK

You are a developer writing a program with Goodfetch's SDK, a library for browser automation (Collie) and crawling (Foxhound). You have not seen this SDK before: its API exists only as type stubs with doc comments, in the read-only folder `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch`. Learn the API from those files.

## Language

TypeScript, as an ES module run by Node 22 or later (top-level `await` and `await using` work). The SDK's packages are `@goodfetch/collie` (and `@goodfetch/collie/testing`), `@goodfetch/foxhound` (and `@goodfetch/foxhound/testing`) and `@goodfetch/collie-client` (and its `/testing`); their stubs are the `.d.ts` files under `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch/typescript/`. Also installed: zod 4.6.7, playwright 1.64.1 and @types/node. Import another local module with a `.js` suffix.

## The task

> Run the crawl of scenario 05 under a name you choose. Kill the process part-way through. Start the program again with the same name: the crawl continues where it stopped, and the dataset ends with every book exactly once. Then retry only the requests that failed in that crawl. Run it a third time with nothing left to do: it must say that nothing was done and why, instead of finishing silently. While the crawl runs, the program can print what is in flight and what is waiting, and why.

The program of the scenario this task builds on is already in your working folder, written against the same SDK: `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-07/05-two-page-crawl.ts`. Import from it; do not change it.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-07`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-07/07-resume-and-retry.ts`.
- Read nothing outside your working folder and the library folder named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-07/check` (tsc 7 with `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax` and `erasableSyntaxOnly`). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
