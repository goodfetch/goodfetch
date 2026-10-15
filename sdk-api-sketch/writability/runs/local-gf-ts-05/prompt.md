# A task for a developer using Goodfetch's SDK

You are a developer writing a program with Goodfetch's SDK, a library for browser automation (Collie) and crawling (Foxhound). You have not seen this SDK before: its API exists only as type stubs with doc comments, in the read-only folder `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch`. Learn the API from those files.

## Language

TypeScript, as an ES module run by Node 22 or later (top-level `await` and `await using` work). The SDK's packages are `@goodfetch/collie` (and `@goodfetch/collie/testing`), `@goodfetch/foxhound` (and `@goodfetch/foxhound/testing`) and `@goodfetch/collie-client` (and its `/testing`); their stubs are the `.d.ts` files under `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch/typescript/`. Also installed: zod 4.6.7, playwright 1.64.1 and @types/node. Import another local module with a `.js` suffix.

## The task

> Crawl https://books.toscrape.com/. From the home page, follow every category link in the side list except the top-level "Books" one. On each category page, follow its book links and its "next" page link. On each book page, record one item with the title, the price (a decimal number of pounds), the number in stock (an integer), the star rating (one to five), the page's address, and the name of the category it was reached from: that name is passed from the category page, not read again on the book page. Stay on the site. Store the items in a named dataset and export it to a JSON file when the crawl ends. Report how many pages succeeded, failed or were skipped, and why each failed or skipped page did.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-gf-ts-05`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-gf-ts-05/05-two-page-crawl.ts`.
- Read nothing outside your working folder and the library folder named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-gf-ts-05/check` (tsc 7 with `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax` and `erasableSyntaxOnly`). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
