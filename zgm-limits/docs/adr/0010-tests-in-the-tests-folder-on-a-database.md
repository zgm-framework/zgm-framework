# ADR-0010: Tests live in the `tests/` folder and check the module from the outside, on a real database

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (`tests/hit.rs`, `src/lib.rs`, `justfile`)

## Context

- **All the counting of a window is in SQL** ([ADR-0006](0006-counting-in-one-sql-statement.md)), so a test without a database checks nothing.
- **The crate was only a program (`main.rs`).** Tests in the `tests/` folder see the crate only as a library, through what is public.
- **Tests kept next to the code,** in `tests` modules of the files in `src/`, make those files grow: a file with the counting can reach several thousand lines.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. The `tests/` folder, through the public entry point** | the code files stay short; the tests use the module the way an application does | they cannot reach inside: time must be given through the public entry point, that is, through the clock from the builder |
| **B. `tests` modules next to the code** | a test can call `repository::hit_window` with any moment | the code files grow by the tests |

## Decision

**A** (decided by the maintainer on 2026-11-10: "write the tests, but in a separate tests folder").

- **The crate is a library with the example program next to it.** `src/lib.rs` exposes `Limits`, `Limit`, `Rule`, `Answer` and `LimitsError`. `repository` stays private.
- **`tests/hit.rs` checks `Attempts::hit`, `tests/reset.rs` checks `reset` and a whole sign-in made of `hit` and `reset`, `tests/cleanup.rs` checks the cleanup loop, and `tests/blocked.rs` checks `Limits::get_blocked` and `unblock`.** Shared helpers are in `tests/common/mod.rs`. `#[sqlx::test]` gives every test a new, empty database with the migrations applied.
- **The tests have their own enum of limits,** `TestLimit`.
- **Time in the tests comes from `TestClock`,** a clock that stands still until the test moves it ([ADR-0011](0011-clock-from-the-builder.md)). The tests check the end of a window to the second and do not wait.
- **The test of attempts sent at once has its own pool of 20 connections and runs 20 rounds of 40 attempts.**
- **The command:** `just test`, with a PostgreSQL 18 server in `DATABASE_URL`.

## Why

- **Short code files are a goal of this crate.**
- **A test of attempts sent at once, with one round and the default pool, did not detect the bug.** Checked by removing the condition on the write on purpose: one round passed despite the broken query, and 20 rounds on 20 connections caught it in 5 runs out of 5.
- **The test of changing the key detects a lookup by the current hash only,** checked the same way.

## Consequences

- **The tests need a running database.** Without `DATABASE_URL` they do not run.
- **SQL in tests stays in the tests:** `tests/hit.rs` reads the table itself to check the counters.

## When to revisit

- Logic appears that cannot be checked through the public entry point.
