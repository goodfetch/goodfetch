# A task for a developer using Scrapy 2.19.0 with FastAPI 0.143.1

You are a developer writing a program with Scrapy 2.19.0 with FastAPI 0.143.1 (and uvicorn 0.54.0).

## Language

Python 3.11, with Scrapy 2.19.0 with FastAPI 0.143.1 (and uvicorn 0.54.0). The packages are installed in `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/env/py/scrapy/lib/python3.11/site-packages`, if you need to read their sources.

## The task

> Write a small web service with one endpoint, `/books?category=<name>`, that crawls that category of https://books.toscrape.com/ (following its "next" pages) when called, and answers with its books as JSON (title and price). Use the language's usual minimal web framework (for example FastAPI, Node's `http` module, axum). Two calls may run at the same time in the same process without interfering, and calling it again later works the same. If the caller disconnects, the crawl stops and releases every connection and browser it used. The library's logs go to the service's own logger, and nothing is printed to standard output. Also call the same crawl from a notebook cell in Python, and from a plain script in TypeScript and Rust.

## How to work

- Your working folder is `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-ld-py-12-scrapy`. Write the complete program to `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-ld-py-12-scrapy/s12_crawl_in_a_service_scrapy.py`.
- Read nothing outside your working folder and the installed packages named above: no other file on this machine, no web, no other project.
- When the program is complete, type-check it by running `/private/tmp/claude-501/-Users-jb-Dev-automan/77744178-6af8-4d82-8e7e-cbe4fbb7ef21/scratchpad/t484/runs/local-ld-py-12-scrapy/check` (pyright in standard mode). Your first check run records your first attempt, so write the whole program before you run it. If the check fails, fix the program and run it again: at most 3 check runs in total. Never run the program itself (nothing is reachable from it).
- Do not edit any other file, and do not create other files in the working folder.
- When you are done, reply with one line only: `DONE <number of check runs> <PASSED or FAILED>`.
