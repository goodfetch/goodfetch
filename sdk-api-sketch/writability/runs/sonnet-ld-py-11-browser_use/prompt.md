# A task for a developer using Browser Use 0.13.11

You are a developer writing a program with Browser Use 0.13.11 (`browser_use`).

## Language

Python 3.11, with Browser Use 0.13.11 (`browser_use`). The packages are installed in `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/env/py/bu/lib/python3.11/site-packages`, if you need to read their sources.

## The task

> On https://books.toscrape.com/, from one instruction, "find the cheapest book rated five stars in the Travel category and give its title and price", reach the answer without writing the steps yourself. Cap the work at 25 steps and 0.20 USD. Return the title and the price as typed fields, together with how the work ended: stopped by a limit, reported as done by the model, or verified by a condition the program states (the page reached is a book page in the Travel category with a five-star rating). Keep the steps taken so that they can be repeated without a model. Stay on books.toscrape.com, whatever the pages suggest. Nothing about the task may be sent to any third party other than the chosen model's provider.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/sonnet-ld-py-11-browser_use`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/sonnet-ld-py-11-browser_use/s11_agent_goal_browser_use.py`.
- Read nothing outside your working folder and the installed packages named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/sonnet-ld-py-11-browser_use/check` (pyright in standard mode). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
