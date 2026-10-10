# SQLite readers and checkpoints

`pnpm sqlite-readers 3` runs 108 cells on the same SQLite artifact:
shared Node, synchronous Rust and Go/cgo; one or four writer processes;
batches of 100 or 1,000 rows; no reader, short read transactions,
or a read snapshot held during all the writes. Each cell
writes 32,000 rows with a 512-byte payload, WAL FULL and eight producers.

The lane measures startup, writing up to the ACKs, PASSIVE checkpoint, release
of the reader, TRUNCATE checkpoint, close and oracle. The raw ACKs are
kept. The short readers hold their transaction for 2 ms and start again
every 10 ms; their counts must remain increasing and bounded.

The long snapshot starts before the first insertion: it must keep
seeing zero rows while all the writers acknowledge their work. A
PASSIVE checkpoint must then keep frames that have not been checkpointed.
After release, TRUNCATE must succeed and bring the WAL back to zero. The oracle
finally reopens the database and checks each record and its integrity.

The coordinator creates the schema once and enables WAL before the concurrent
connections. The 31 errors of the first matrix came from this initialization
race, before any insertion; they are lab diagnostics,
not measurements of writer contention. The writers remain independent
processes that genuinely compete for the same lock during the measurement.

These invariants make the experiment useful even when the write window
is short. The throughputs of this lane are not a sustained qualification.
The probe includes the already-running coordinator and the writers; it subtracts
the coordinator's initial CPU/I/O counters. Its total footprint may
include allocations from previous campaigns and does not rank the intrinsic
memory cost of the three languages.

[SQLite WAL](https://www.sqlite.org/wal.html) explains the single active writer,
the coexistence of reads/writes and the checkpoint stalling behind a
long snapshot. This lane concretely verifies these properties and their cost
in the lab's bindings. It tests neither a network filesystem, nor power
loss, nor hardware durability beyond the declared WAL FULL contract.
