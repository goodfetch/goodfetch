# A task for a developer using Crawlee 3.18.2

You are a developer writing a program with Crawlee 3.18.2 (`crawlee`).

## Language

TypeScript, as an ES module run by Node 22 or later (top-level `await` works), with Crawlee 3.18.2 (`crawlee`). The packages are installed in `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-ts-07-crawlee/node_modules`, with their type definitions, and @types/node. Import another local module with a `.js` suffix.

## The task

> Run the crawl of scenario 05 under a name you choose. Kill the process part-way through. Start the program again with the same name: the crawl continues where it stopped, and the dataset ends with every book exactly once. Then retry only the requests that failed in that crawl. Run it a third time with nothing left to do: it must say that nothing was done and why, instead of finishing silently. While the crawl runs, the program can print what is in flight and what is waiting, and why.

The program of the scenario this task builds on is already in your working folder, written with the same library: `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-ts-07-crawlee/05-two-page-crawl.crawlee.ts`. Import from it; do not change it.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-ts-07-crawlee`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-ts-07-crawlee/07-resume-and-retry.crawlee.ts`.
- Read nothing outside your working folder and the installed packages named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-ld-ts-07-crawlee/check` (tsc 7 with `strict` (and `skipLibCheck`)). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
