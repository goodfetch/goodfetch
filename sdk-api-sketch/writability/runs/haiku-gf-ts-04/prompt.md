# A task for a developer using Goodfetch's SDK

You are a developer writing a program with Goodfetch's SDK, a library for browser automation (Collie) and crawling (Foxhound). You have not seen this SDK before: its API exists only as type stubs with doc comments, in the read-only folder `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch`. Learn the API from those files.

## Language

TypeScript, as an ES module run by Node 22 or later (top-level `await` and `await using` work). The SDK's packages are `@goodfetch/collie` (and `@goodfetch/collie/testing`), `@goodfetch/foxhound` (and `@goodfetch/foxhound/testing`) and `@goodfetch/collie-client` (and its `/testing`); their stubs are the `.d.ts` files under `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/sketch/typescript/`. Also installed: zod 4.6.7, playwright 1.64.1 and @types/node. Import another local module with a `.js` suffix.

## The task

> Log in to https://quotes.toscrape.com/login once with a person at the keyboard. Open a visible browser at the login page, fill the username from the program and the password from a secret the program never holds in clear (an environment variable or the system's keychain), then wait for the person to check the form and press Login, with no time limit while waiting and a way to cancel. Keep the logged-in state under a name. In a second, headless run, reuse that state to open https://quotes.toscrape.com/ and confirm that the page shows a Logout link. If the kept state has expired, say so instead of failing on a missing element. The password must not appear in any log, trace or saved file.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-04`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-04/04-login-human-pause.ts`.
- Read nothing outside your working folder and the library folder named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/haiku-gf-ts-04/check` (tsc 7 with `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax` and `erasableSyntaxOnly`). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
