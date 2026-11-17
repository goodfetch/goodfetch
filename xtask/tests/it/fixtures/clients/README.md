# Clients workspace

A workspace whose Client conformance case list (`conformance/clients/`) is
implemented on each surface it lists, in each of that surface's modes, by
tests in each Product's executable crate (ADR 0069), for `xtask clients
check`. One case lists only `sdk-typescript` and one only `sdk-python`,
whose tests the check reads from the runners' listings in `listings/`, as
`xtask journeys listings` writes them, and only notes without them.
