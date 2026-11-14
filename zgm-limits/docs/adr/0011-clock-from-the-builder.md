# ADR-0011: The moment of an attempt comes from the module's clock, which a test replaces in the builder

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (`src/clock.rs`, `src/limits.rs`, `tests/hit.rs`)

## Context

- **The window query gets the moment of the attempt as a parameter** ([ADR-0006](0006-counting-in-one-sql-statement.md)), and `hit` took it from `Utc::now()`.
- **The tests live in `tests/` and see only the public entry point** ([ADR-0010](0010-tests-in-the-tests-folder-on-a-database.md)). The end of a window could be checked only on a half-second window, by really waiting.
- **`zgm-challenges`, another crate by the same author, has a `Clock` trait with one method, `now()`,** and the system clock as the default in the builder.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. A `Clock` trait and a `.clock(...)` step in the builder, the system clock by default** | a test moves time without waiting and checks the end of a window to the second; the same pattern as in the rest of the repository | one more trait and one more field |
| **B. `hit` takes the moment as an argument** | no trait | every application passes the time with every attempt and can pass a wrong one |
| **C. The database clock in the query** | one clock for all servers | the tests still have to really wait |

## Decision

**A** (decided by the maintainer on 2026-11-10: "then go ahead with the clock").

- **`Clock` has one method, `now()`.** `SystemClock` reads the system clock.
- **The builder takes `SystemClock` unless somebody gives it another clock.** The application sets nothing.
- **The clock is read after a connection is taken from the pool,** not before. So waiting for a connection when the pool is exhausted does not make the moment of the attempt older.
- **`hit` asks the clock once per attempt and cuts the moment to microseconds,** because that is what PostgreSQL keeps. A moment written to the database and read back is then the same moment.
- **The tests have `TestClock`,** which stands at 12:00 and moves only through `advance`.

## Why

- **Windows last an hour or a day,** so without replacing the clock their end cannot be checked.
- **Checked with a test:** with three attempts at 12:00, 12:01 and 12:02, the refusal gives the end of the window as exactly 13:00; at 12:59:59 the window is still full, and at 13:00:00 a new one opens, until 14:00.

## Consequences

- **The application servers count by their own clocks,** so a difference between their clocks moves the start and the end of a window by that difference: an attempt on a server whose clock runs fast ends the window earlier.
- **The cleanup does not shorten blocks because of this,** because it deletes rows only an hour after their end ([ADR-0015](0015-cleanup-in-a-job-loop.md)).
- **`tokio` no longer needs the `time` feature in the tests.**
