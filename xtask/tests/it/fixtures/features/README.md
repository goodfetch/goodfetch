# Features workspace

A workspace whose tests declare Features from every place a test sits, with
the `features/` files those Features come from (ADRs 0072 and 0075). Its
inventory is written by hand: `xtask features check` reads it as committed.
