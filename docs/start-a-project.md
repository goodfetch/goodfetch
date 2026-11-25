---
title: Start a project
description: Write a new project from a starter with `goodfetch-collie init`, in TypeScript, Python or Rust, and run its program and its test.
products: [collie]
---

# Start a project

`goodfetch-collie init` writes a starter into a folder: a small project
whose program opens a page in a browser and prints its title, with a test
that runs the program on Collie's testing kit. The `browser` starter comes
in TypeScript, Python and Rust.

```sh
goodfetch-collie init my-project --starter browser --lang typescript
```

Left out on a terminal, the starter and the language are asked. A file
already in the folder is never overwritten: `init` lists it as skipped and
writes the others. The project's name is the folder's, and its manifest
asks for the Collie version that wrote it.

## TypeScript

The project needs Node 24.2 or later. Install it, run its test and start
its program:

```sh
npm install
npm test
npm start -- https://example.com/
```

Its program, `src/main.ts`:

```ts
// Prints the title of the page its argument names, the Proving ground's
// when it has none: `npm start -- https://example.com/`.
import { Collie } from "@goodfetch/collie";

/** Opens `url` in a browser page of a new Run on `collie` and answers the page's title. */
export async function readTitle(collie: Collie, url: string): Promise<string> {
  const run = await collie.start();
  const page = await run.newPage();
  await page.goto(url);
  const title = await page.evaluate("() => document.title");
  await run.finish();
  return String(title);
}

// Started as the program, not imported by its test.
if (import.meta.main) {
  const url = process.argv[2] ?? "https://proving-ground.goodfetch.dev/";
  await using collie = await Collie.connect();
  console.log(await readTitle(collie, url));
}
```

## Python

The project is a uv project for Python 3.11 or later:

```sh
uv sync
uv run pytest
uv run main.py https://example.com/
```

Its program, `main.py`:

```python
"""Prints the title of the page its argument names, else the Proving ground's.

Run it with `uv run main.py https://example.com/`.
"""

import sys

from goodfetch_collie import Collie


def read_title(collie: Collie, url: str) -> str:
    """Opens `url` in a browser page of a new Run and answers the page's title."""
    run = collie.start()
    page = run.new_page()
    page.goto(url)
    title = page.evaluate("() => document.title")
    run.finish()
    return str(title)


# Started as the program, not imported by its test.
if __name__ == "__main__":
    url = sys.argv[1] if len(sys.argv) > 1 else "https://proving-ground.goodfetch.dev/"
    with Collie.connect() as collie:
        sys.stdout.write(f"{read_title(collie, url)}\n")
```

## Rust

The project builds Collie's crate from source, which needs the tools
[Install](install.md) lists:

```sh
cargo test
cargo run -- https://example.com/
```

Its program, `src/main.rs`, with its test in `src/tests.rs`:

```rust
//! Prints the title of the page its argument names, the Proving ground's
//! when it has none: `cargo run -- https://example.com/`.

use goodfetch_collie::{Collie, ConnectOptions, Error};

/// Opens `url` in a browser page of a new Run on `collie` and answers the
/// page's title.
async fn read_title(collie: &Collie, url: &str) -> Result<String, Error> {
    let run = collie.start().await?;
    let page = run.new_page().await?;
    page.goto(url).await?;
    let title = page.evaluate("() => document.title").await?;
    run.finish().await?;
    Ok(title
        .as_ref()
        .and_then(|title| title.as_str())
        .unwrap_or_default()
        .to_owned())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://proving-ground.goodfetch.dev/".to_owned());
    let title = Collie::scope(ConnectOptions::default(), async |collie| {
        read_title(collie, &url).await
    })
    .await?;
    println!("{title}");
    Ok(())
}

#[cfg(test)]
mod tests;
```
