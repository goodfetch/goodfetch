---
name: {{executable}}-cli
description: "Drives a browser from a shell with the {{executable}} command line: starts a Run, opens a page, navigates it, clicks, fills and reads it, then finishes the Run and reads its report. Use when a task needs a web page loaded, acted on or read from a terminal or a script."
---
# Driving a browser with the {{executable}} command line

Each command makes one call and prints its answer. Add `--json` to read the answer as one JSON object; the examples below do.

## Where the calls run

A Run outlives the command that starts it, so `{{executable}} run start` reaches the Daemon, starting it when none runs, and every later command reaches the same Daemon while it runs. `{{executable}} daemon status` says whether it runs. A command given `--attached` runs on an engine in its own process, which stops with the command and stops the Runs it started: never pass it to a command of a Run that must last.

## A Run, step by step

Start a Run. Its answer's `run_id` names the Run in every later command:

```console
$ {{executable}} run start --json
```

Open a page in it. The first page launches the Run's browser, and the answer's `page_id` names the page:

```console
$ {{executable}} run new-page <run-id> --json
```

Navigate the page. A 404 or a 500 is an answer, not a failure; a navigation that reaches no document fails with `navigation_failed`:

```console
$ {{executable}} page goto <page-id> --url https://example.com/ --json
```

Read named fields of the page's document in one call. Each field finds its element with a CSS or XPath locator; a field the page does not give is null, and the answer's `unread` says why:

```console
$ {{executable}} page read <page-id> --fields '{"title":{"locator":{"kind":"css","selector":"h1"}}}' --json
```

Act on an element. A locator names its page and a CSS selector, and the action waits until the element is unique, visible, enabled and stable:

```console
$ {{executable}} locator click --locator '{"page_id":"<page-id>","selector":"a.next"}' --json
$ {{executable}} locator fill --locator '{"page_id":"<page-id>","selector":"input[name=q]"}' --value 'browser automation' --json
$ {{executable}} locator press --locator '{"page_id":"<page-id>","selector":"input[name=q]"}' --key Enter --json
```

Finish the Run. Its answer is the Run report; `{{executable}} run report` reads it again later:

```console
$ {{executable}} run finish <run-id> --json
$ {{executable}} run report <run-id> --json
```

## When a Run stops short

- `{{executable}} run usage <run-id>` answers what the Run used and what each of its Limits leaves.
- `{{executable}} run stop <run-id>` stops a Run at its next safe point, resumable by its Run ID: a later `{{executable}} run start --run-id <run-id>` resumes it.
- `{{executable}} needs-you list` lists what waits for a person across every Run.
- A Limit reached is a person's to raise: stop, and ask the person to run `{{executable}} usage raise` at their own terminal.
